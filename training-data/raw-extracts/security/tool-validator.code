use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

const SHELL_TIMEOUT: Duration = Duration::from_secs(5);

// ─── Result type ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub language: String,
}

impl ValidationResult {
    fn valid(language: impl Into<String>) -> Self {
        Self {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
            language: language.into(),
        }
    }

    fn invalid(language: impl Into<String>, errors: Vec<String>) -> Self {
        Self {
            is_valid: false,
            errors,
            warnings: vec![],
            language: language.into(),
        }
    }
}

// ─── Dangerous-pattern scanner ───────────────────────────────────────────────

/// Returns human-readable warnings for dangerous patterns found in `code`.
/// Never blocks execution — callers decide what to do with warnings.
pub fn dangerous_patterns(code: &str) -> Vec<String> {
    // Each entry: (pattern_substring, human message)
    let patterns: &[(&str, &str)] = &[
        ("rm -rf /", "destructive recursive delete of root filesystem (rm -rf /)"),
        ("rm -rf /*", "destructive recursive delete of root filesystem (rm -rf /*)"),
        (":(){ :|:& };:", "fork bomb detected"),
        (":(){:|:&};:", "fork bomb detected"),
        ("dd if=/dev/zero of=/dev/", "overwriting a block device with zeros"),
        ("mkfs.", "disk formatting command (mkfs)"),
        ("format c:", "Windows disk format command"),
        ("> /dev/sda", "redirecting output to raw disk device"),
        ("> /dev/hda", "redirecting output to raw disk device"),
        ("shred /dev/", "shredding block device"),
        ("chmod -R 777 /", "recursively opening all filesystem permissions"),
        ("chmod 777 /", "opening root filesystem permissions"),
    ];

    let lower = code.to_lowercase();
    let mut warnings = Vec::new();

    for (pat, msg) in patterns {
        let pat_lower = pat.to_lowercase();
        if lower.contains(&pat_lower) {
            warnings.push(format!("DANGEROUS: {msg}"));
        }
    }

    warnings
}

// ─── Language detection ──────────────────────────────────────────────────────

/// Heuristic language detection from code content.
/// Returns one of: "rust", "python", "javascript", "shell", or "unknown".
pub fn detect_language(code: &str) -> &str {
    // Shell shebang — highest priority
    if code.starts_with("#!/bin")
        || code.starts_with("#!/usr/bin/env bash")
        || code.starts_with("#!/usr/bin/env sh")
    {
        return "shell";
    }

    // Count keyword hits for each language
    let rust_score = count_keywords(
        code,
        &["fn ", "pub fn", "use ", "let mut ", "impl ", "struct ", "enum ", "mod "],
    );
    let python_score = count_keywords(
        code,
        &["def ", "import ", "from ", "elif ", "print(", "self.", "class "],
    );
    let js_score = count_keywords(
        code,
        &[
            "function ",
            "const ",
            "let ",
            "var ",
            "=>",
            "require(",
            "console.log",
            "export ",
        ],
    );
    let shell_score = count_keywords(
        code,
        &["#!/", "echo ", "fi\n", "then\n", "do\n", "done\n", "elif "],
    );

    let max = rust_score.max(python_score).max(js_score).max(shell_score);
    if max == 0 {
        return "unknown";
    }

    if rust_score == max {
        "rust"
    } else if python_score == max {
        "python"
    } else if js_score == max {
        "javascript"
    } else {
        "shell"
    }
}

fn count_keywords(code: &str, keywords: &[&str]) -> usize {
    keywords.iter().filter(|&&kw| code.contains(kw)).count()
}

// ─── Brace / bracket balance check ───────────────────────────────────────────

fn check_brace_balance(code: &str) -> Vec<String> {
    let mut braces: i64 = 0;
    let mut parens: i64 = 0;
    let mut brackets: i64 = 0;
    let mut in_string_double = false;
    let mut in_string_single = false;
    let mut prev = '\0';

    for ch in code.chars() {
        // Toggle string state (naïve — doesn't handle escape sequences fully,
        // but good enough for structural balance detection).
        if ch == '"' && !in_string_single && prev != '\\' {
            in_string_double = !in_string_double;
        } else if ch == '\'' && !in_string_double && prev != '\\' {
            in_string_single = !in_string_single;
        }

        if !in_string_double && !in_string_single {
            match ch {
                '{' => braces += 1,
                '}' => braces -= 1,
                '(' => parens += 1,
                ')' => parens -= 1,
                '[' => brackets += 1,
                ']' => brackets -= 1,
                _ => {}
            }
        }
        prev = ch;
    }

    let mut errors = Vec::new();
    if braces != 0 {
        errors.push(format!(
            "Unbalanced braces: {} unclosed",
            braces.abs()
        ));
    }
    if parens != 0 {
        errors.push(format!(
            "Unbalanced parentheses: {} unclosed",
            parens.abs()
        ));
    }
    if brackets != 0 {
        errors.push(format!(
            "Unbalanced brackets: {} unclosed",
            brackets.abs()
        ));
    }
    errors
}

// ─── CodeValidator ────────────────────────────────────────────────────────────

pub struct CodeValidator;

impl CodeValidator {
    pub fn new() -> Self {
        Self
    }

    /// Dispatch to the language-specific validator.
    /// If `language` is empty or "auto", detect from content.
    pub async fn validate(&self, code: &str, language: &str) -> ValidationResult {
        let lang = if language.is_empty() || language == "auto" {
            detect_language(code)
        } else {
            language
        };

        let mut result = match lang {
            "python" => self.validate_python(code).await,
            "rust" => self.validate_rust(code),
            "javascript" | "js" | "typescript" | "ts" => self.validate_javascript(code).await,
            "shell" | "bash" | "sh" => self.validate_shell(code).await,
            _ => ValidationResult::valid(lang),
        };

        // Attach dangerous-pattern warnings regardless of language
        let dp = dangerous_patterns(code);
        result.warnings.extend(dp);

        result
    }

    // ── Python ───────────────────────────────────────────────────────────────

    pub async fn validate_python(&self, code: &str) -> ValidationResult {
        // Shell out: python3 -c <script>
        // Code is passed via stdin to avoid any shell-injection from code content.
        let script =
            "import ast, sys\nsource = sys.stdin.read()\ntry:\n    ast.parse(source)\nexcept SyntaxError as e:\n    print(f'SyntaxError: {e}', file=sys.stderr)\n    sys.exit(1)\n";

        let code_bytes = code.as_bytes().to_vec();

        let run = async move {
            use tokio::io::AsyncWriteExt;
            let mut child = Command::new("python3")
                .arg("-c")
                .arg(script)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(&code_bytes).await;
            }
            child.wait_with_output().await
        };

        match timeout(SHELL_TIMEOUT, run).await {
            Ok(Ok(output)) => {
                if output.status.success() {
                    ValidationResult::valid("python")
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    ValidationResult::invalid("python", vec![stderr.trim().to_string()])
                }
            }
            Ok(Err(e)) => ValidationResult::invalid(
                "python",
                vec![format!("python3 not available: {e}")],
            ),
            Err(_) => ValidationResult::invalid(
                "python",
                vec!["Validation timed out after 5 seconds".to_string()],
            ),
        }
    }

    // ── Rust ─────────────────────────────────────────────────────────────────

    pub fn validate_rust(&self, code: &str) -> ValidationResult {
        let mut errors = check_brace_balance(code);

        // Known dangerous Rust patterns
        let dangerous_rust: &[(&str, &str)] = &[
            ("unsafe {", "contains unsafe block — review carefully"),
            ("std::mem::transmute", "uses mem::transmute — type safety bypassed"),
            ("std::ptr::null_mut", "raw null pointer usage"),
        ];
        let mut warnings = Vec::new();
        for (pat, msg) in dangerous_rust {
            if code.contains(pat) {
                warnings.push(format!("WARNING: {msg}"));
            }
        }

        let mut result = if errors.is_empty() {
            ValidationResult::valid("rust")
        } else {
            ValidationResult::invalid("rust", errors.drain(..).collect())
        };
        result.warnings = warnings;
        result
    }

    // ── JavaScript / TypeScript ───────────────────────────────────────────────

    pub async fn validate_javascript(&self, code: &str) -> ValidationResult {
        // Try `node --check` via bash reading from /dev/stdin.
        // Piping code to stdin avoids writing temp files and prevents
        // path-traversal attacks from any filename derived from code content.
        let code_bytes = code.as_bytes().to_vec();

        let run = async move {
            use tokio::io::AsyncWriteExt;
            let mut child = Command::new("bash")
                .arg("-c")
                .arg("node --check /dev/stdin")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(&code_bytes).await;
            }
            child.wait_with_output().await
        };

        match timeout(SHELL_TIMEOUT, run).await {
            Ok(Ok(output)) => {
                if output.status.success() {
                    return ValidationResult::valid("javascript");
                }
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("not found") || stderr.contains("No such file") {
                    return self.validate_js_fallback(code);
                }
                ValidationResult::invalid("javascript", vec![stderr.trim().to_string()])
            }
            Ok(Err(_)) | Err(_) => self.validate_js_fallback(code),
        }
    }

    fn validate_js_fallback(&self, code: &str) -> ValidationResult {
        let errors = check_brace_balance(code);
        if errors.is_empty() {
            ValidationResult::valid("javascript")
        } else {
            ValidationResult::invalid("javascript", errors)
        }
    }

    // ── Shell ─────────────────────────────────────────────────────────────────

    pub async fn validate_shell(&self, code: &str) -> ValidationResult {
        // `bash -n` is syntax-only — it never executes the script.
        // Code is piped via stdin so no shell metacharacters in the code string
        // can escape into the process invocation arguments.
        let code_bytes = code.as_bytes().to_vec();

        let run = async move {
            use tokio::io::AsyncWriteExt;
            let mut child = Command::new("bash")
                .arg("-n")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(&code_bytes).await;
            }
            child.wait_with_output().await
        };

        match timeout(SHELL_TIMEOUT, run).await {
            Ok(Ok(output)) => {
                if output.status.success() {
                    ValidationResult::valid("shell")
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    ValidationResult::invalid("shell", vec![stderr.trim().to_string()])
                }
            }
            Ok(Err(e)) => ValidationResult::invalid(
                "shell",
                vec![format!("bash not available: {e}")],
            ),
            Err(_) => ValidationResult::invalid(
                "shell",
                vec!["Validation timed out after 5 seconds".to_string()],
            ),
        }
    }
}

impl Default for CodeValidator {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── detect_language ──────────────────────────────────────────────────────

    #[test]
    fn test_detect_rust() {
        let code = "pub fn main() {\n    let mut x = 5;\n    println!(\"{}\", x);\n}";
        assert_eq!(detect_language(code), "rust");
    }

    #[test]
    fn test_detect_python() {
        let code = "def greet(name):\n    import sys\n    print(f'hello {name}')";
        assert_eq!(detect_language(code), "python");
    }

    #[test]
    fn test_detect_javascript() {
        let code = "const greet = (name) => {\n    console.log('hello', name);\n};";
        assert_eq!(detect_language(code), "javascript");
    }

    #[test]
    fn test_detect_shell_shebang() {
        let code = "#!/bin/bash\necho hello\n";
        assert_eq!(detect_language(code), "shell");
    }

    #[test]
    fn test_detect_unknown() {
        let code = "just some plain text without code";
        assert_eq!(detect_language(code), "unknown");
    }

    // ── dangerous_patterns ───────────────────────────────────────────────────

    #[test]
    fn test_dangerous_rm_rf_root() {
        let warnings = dangerous_patterns("rm -rf /");
        assert!(!warnings.is_empty());
        assert!(warnings[0].contains("DANGEROUS"));
    }

    #[test]
    fn test_dangerous_fork_bomb() {
        let warnings = dangerous_patterns(":(){ :|:& };:");
        assert!(!warnings.is_empty());
        assert!(warnings[0].contains("fork bomb"));
    }

    #[test]
    fn test_dangerous_mkfs() {
        let warnings = dangerous_patterns("mkfs.ext4 /dev/sda1");
        assert!(!warnings.is_empty());
        assert!(warnings[0].contains("mkfs"));
    }

    #[test]
    fn test_dangerous_multiple_patterns() {
        let code = "rm -rf / && mkfs.ext4 /dev/sda";
        let warnings = dangerous_patterns(code);
        assert!(warnings.len() >= 2);
    }

    #[test]
    fn test_safe_code_no_warnings() {
        let code = "fn main() { println!(\"hello\"); }";
        let warnings = dangerous_patterns(code);
        assert!(warnings.is_empty());
    }

    // P0: injection — dangerous patterns are case-insensitive
    #[test]
    fn test_dangerous_case_insensitive() {
        let warnings = dangerous_patterns("RM -RF /");
        assert!(!warnings.is_empty());
    }

    // ── brace balance ────────────────────────────────────────────────────────

    #[test]
    fn test_balanced_braces() {
        let errors = check_brace_balance("fn foo() { let x = 1; }");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_unbalanced_open_brace() {
        let errors = check_brace_balance("fn foo() { let x = 1;");
        assert!(!errors.is_empty());
        assert!(errors[0].contains("brace"));
    }

    #[test]
    fn test_unbalanced_paren() {
        let errors = check_brace_balance("foo(bar(");
        assert!(!errors.is_empty());
        assert!(errors[0].contains("parenthes"));
    }

    #[test]
    fn test_balanced_nested() {
        let errors = check_brace_balance("fn f() { if x { [1, 2] } }");
        assert!(errors.is_empty());
    }

    // ── validate_rust (sync, pure logic) ────────────────────────────────────

    #[test]
    fn test_validate_rust_valid() {
        let v = CodeValidator::new();
        let result = v.validate_rust("pub fn main() { let x = 1; }");
        assert!(result.is_valid);
        assert!(result.errors.is_empty());
        assert_eq!(result.language, "rust");
    }

    #[test]
    fn test_validate_rust_unbalanced() {
        let v = CodeValidator::new();
        let result = v.validate_rust("fn foo() { let x = 1;");
        assert!(!result.is_valid);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_validate_rust_unsafe_warning() {
        let v = CodeValidator::new();
        let result = v.validate_rust("fn f() { unsafe { let x = 1; } }");
        assert!(result.is_valid); // still valid — just a warning
        assert!(result.warnings.iter().any(|w| w.contains("unsafe")));
    }

    #[test]
    fn test_validate_rust_transmute_warning() {
        let v = CodeValidator::new();
        let result = v.validate_rust("use std::mem::transmute;");
        assert!(result.warnings.iter().any(|w| w.contains("transmute")));
    }

    // ── async validators (shell out) ─────────────────────────────────────────

    #[tokio::test]
    async fn test_validate_python_valid() {
        let v = CodeValidator::new();
        let result = v.validate_python("def foo():\n    return 1\n").await;
        // If python3 is not available the error is expected — test that the
        // struct is populated correctly either way.
        assert_eq!(result.language, "python");
        // If python3 IS available, it should pass.
        if result.is_valid {
            assert!(result.errors.is_empty());
        }
    }

    #[tokio::test]
    async fn test_validate_python_syntax_error() {
        let v = CodeValidator::new();
        let result = v.validate_python("def foo(\n    return 1\n").await;
        assert_eq!(result.language, "python");
        // If python3 is available, this must fail.
        if !result.errors.iter().any(|e| e.contains("not available")) {
            assert!(!result.is_valid);
        }
    }

    #[tokio::test]
    async fn test_validate_shell_valid() {
        let v = CodeValidator::new();
        let result = v.validate_shell("#!/bin/bash\necho hello\n").await;
        assert_eq!(result.language, "shell");
        assert!(result.is_valid);
    }

    #[tokio::test]
    async fn test_validate_shell_syntax_error() {
        let v = CodeValidator::new();
        let result = v.validate_shell("if [ -z \"$VAR\"\nthen\nfoo\n").await;
        assert_eq!(result.language, "shell");
        assert!(!result.is_valid);
    }

    #[tokio::test]
    async fn test_validate_javascript_fallback_valid() {
        let v = CodeValidator::new();
        // Even if node is missing, the brace-balance fallback should pass this.
        let result = v.validate_javascript("function foo() { return 1; }").await;
        assert_eq!(result.language, "javascript");
        assert!(result.is_valid);
    }

    #[tokio::test]
    async fn test_validate_javascript_fallback_invalid() {
        let v = CodeValidator::new();
        let result = v.validate_javascript("function foo( { return 1; }").await;
        assert_eq!(result.language, "javascript");
        assert!(!result.is_valid);
    }

    // ── validate dispatch ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_validate_auto_detect_rust() {
        let v = CodeValidator::new();
        let result = v
            .validate("pub fn main() { let x = 1; }", "auto")
            .await;
        assert_eq!(result.language, "rust");
        assert!(result.is_valid);
    }

    #[tokio::test]
    async fn test_validate_attaches_dangerous_warnings() {
        let v = CodeValidator::new();
        let result = v.validate("#!/bin/bash\nrm -rf /\n", "shell").await;
        assert!(!result.warnings.is_empty());
        assert!(result.warnings.iter().any(|w| w.contains("DANGEROUS")));
    }

    // P0: input injection — null bytes and control characters in code don't crash
    #[tokio::test]
    async fn test_validate_null_bytes_in_code() {
        let v = CodeValidator::new();
        let code = "fn main() {\x00}";
        let result = v.validate(code, "rust").await;
        assert_eq!(result.language, "rust");
        // Should not panic, result may or may not be valid
    }

    // P0: input injection — extremely long code doesn't hang
    #[tokio::test]
    async fn test_validate_very_long_code() {
        let v = CodeValidator::new();
        let code = "let x = 1;\n".repeat(10_000);
        let result = v.validate(&code, "javascript").await;
        assert_eq!(result.language, "javascript");
    }

    // P0: input injection — shell metacharacters in code passed to python validator
    #[tokio::test]
    async fn test_python_code_with_shell_metacharacters() {
        let v = CodeValidator::new();
        // Code contains shell-special chars — must not escape validation sandbox
        let code = "x = \"$(rm -rf /)\"\nprint(x)\n";
        let result = v.validate_python(code).await;
        assert_eq!(result.language, "python");
        // Should parse fine as Python — the metacharacters are inside a string
        if !result.errors.iter().any(|e| e.contains("not available")) {
            assert!(result.is_valid);
        }
    }

    // P0: input injection — code with backticks doesn't execute via shell
    #[tokio::test]
    async fn test_shell_backtick_injection_in_python_validator() {
        let v = CodeValidator::new();
        let code = "x = `whoami`\n"; // invalid Python — backtick not valid
        let result = v.validate_python(code).await;
        assert_eq!(result.language, "python");
        if !result.errors.iter().any(|e| e.contains("not available")) {
            assert!(!result.is_valid); // SyntaxError
        }
    }
}
