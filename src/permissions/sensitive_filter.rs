//! Sensitive data redaction for content destined for persistence or logs.
//!
//! Implements the `SensitiveDataFilter` utility required by the FolkTech
//! engineering rules (CAT 3 — Sensitive Data Exposure / CAT 9 — Memory &
//! Persistence). Applied at every persistence boundary so credentials
//! pasted into the conversation are not permanently archived in
//! `sessions.db` and re-injected into future system prompts.
//!
//! ## Threat model
//!
//! Forge's session storage routinely captures whatever the user pastes into
//! the chat — code samples, error logs, and occasionally raw credentials
//! ("here's the API key that's failing"). Without a redaction layer, those
//! credentials persist forever, get re-loaded on `forge --resume`, and may
//! end up in cloud-API calls if the user later switches to an API backend.
//!
//! The filter is preventative, not perfect: regex-based redaction is best-
//! effort. We catch the common credential patterns the audit explicitly
//! called out (P0 #9) and leave room for additions as new attack vectors
//! emerge.
//!
//! ## What we redact (covered patterns)
//!
//! - OpenAI / Anthropic-style API keys: `sk-...` and `sk-ant-...`
//! - Slack tokens: `xoxb-...`, `xoxp-...`, `xoxs-...`, `xoxa-...`, `xoxr-...`
//! - GitHub tokens: `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_`, `github_pat_`
//! - AWS access key IDs: `AKIA[A-Z0-9]{16}`
//! - Google API keys: `AIza[A-Za-z0-9\-_]{35}`
//! - Bearer tokens: `Authorization: Bearer ...`, `bearer <token>`
//! - Inline `password=...` / `password: ...` (best-effort)
//! - PEM private key blocks: `-----BEGIN ... PRIVATE KEY-----...-----END...`
//!
//! ## What we deliberately do NOT redact
//!
//! - Email addresses: legitimately appear in code, comments, configs
//! - 16-digit numbers: false-positive risk on UUIDs, hashes, version IDs
//! - Generic high-entropy strings: too many false positives
//!
//! ## Output format
//!
//! Each match is replaced with a label that preserves enough structure for
//! a developer reading the redacted text to recognize what was there
//! without exposing the secret:
//!   `sk-A1B2...` → `sk-[REDACTED]`
//!   `Bearer abcdef` → `Bearer [REDACTED]`
//!   `-----BEGIN RSA PRIVATE KEY-----...` → `[REDACTED PRIVATE KEY]`

use std::sync::OnceLock;

use regex::Regex;

/// All redaction patterns. Compiled once on first use via OnceLock.
fn patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            // PEM private keys — match first so they don't get partially
            // mangled by other patterns. Multi-line.
            (
                Regex::new(r"(?s)-----BEGIN [A-Z ]+ PRIVATE KEY-----.+?-----END [A-Z ]+ PRIVATE KEY-----").unwrap(),
                "[REDACTED PRIVATE KEY]",
            ),
            // Anthropic-style: sk-ant-... (must come before generic sk- to
            // avoid the generic match swallowing the prefix).
            (
                Regex::new(r"sk-ant-[A-Za-z0-9_\-]{20,}").unwrap(),
                "sk-ant-[REDACTED]",
            ),
            // OpenAI-style: sk-... (≥20 char tail to avoid matching things
            // like "sk-1234" that aren't credentials).
            (
                Regex::new(r"sk-[A-Za-z0-9_\-]{20,}").unwrap(),
                "sk-[REDACTED]",
            ),
            // Slack tokens
            (
                Regex::new(r"xox[bpsar]-[A-Za-z0-9\-]{20,}").unwrap(),
                "xox-[REDACTED]",
            ),
            // GitHub PATs
            (
                Regex::new(r"ghp_[A-Za-z0-9]{36,}").unwrap(),
                "ghp_[REDACTED]",
            ),
            (
                Regex::new(r"gho_[A-Za-z0-9]{36,}").unwrap(),
                "gho_[REDACTED]",
            ),
            (
                Regex::new(r"ghu_[A-Za-z0-9]{36,}").unwrap(),
                "ghu_[REDACTED]",
            ),
            (
                Regex::new(r"ghs_[A-Za-z0-9]{36,}").unwrap(),
                "ghs_[REDACTED]",
            ),
            (
                Regex::new(r"ghr_[A-Za-z0-9]{36,}").unwrap(),
                "ghr_[REDACTED]",
            ),
            (
                Regex::new(r"github_pat_[A-Za-z0-9_]{60,}").unwrap(),
                "github_pat_[REDACTED]",
            ),
            // AWS access key ID
            (
                Regex::new(r"AKIA[A-Z0-9]{16}").unwrap(),
                "AKIA[REDACTED]",
            ),
            // Google API key
            (
                Regex::new(r"AIza[A-Za-z0-9_\-]{35}").unwrap(),
                "AIza[REDACTED]",
            ),
            // Bearer token in Authorization header style.
            (
                Regex::new(r"(?i)\b(bearer)\s+[A-Za-z0-9._\-]{20,}").unwrap(),
                "$1 [REDACTED]",
            ),
            // password=value or password: value (best-effort).
            // Matches up to whitespace or end-of-line; preserves the key
            // name for grep-ability.
            (
                Regex::new(r#"(?i)\b(password|passwd|pwd)\s*[:=]\s*["']?([^\s"',]{3,})"#).unwrap(),
                "$1=[REDACTED]",
            ),
        ]
    })
}

/// Apply all redaction patterns to the input text. Idempotent: running
/// twice on already-redacted text changes nothing further. Returns a new
/// `String` so callers can feed it to persistence layers without
/// modifying their inputs.
pub fn redact_sensitive(text: &str) -> String {
    let mut out = text.to_string();
    for (pattern, replacement) in patterns() {
        out = pattern.replace_all(&out, *replacement).to_string();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE on test fixtures:
    // GitHub push protection detects strings that match real-credential
    // patterns. Test fixtures use X-padded clearly-fake values (e.g.
    // `sk-ant-XXXXXXXXXXXXXXXXXXXX`) so the scanner doesn't reject the push
    // while still exercising the regex.

    /// SECURITY (CAT 3 / CAT 9):
    /// Anthropic-style keys must be redacted, NOT survive into persistence.
    #[test]
    fn test_redacts_anthropic_api_key() {
        let fixture = "sk-ant-XXXXXXXXXXXXXXXXXXXXXXXXXXXX";
        let pasted = format!("my key is {fixture}");
        let safe = redact_sensitive(&pasted);
        assert!(!safe.contains(fixture));
        assert!(safe.contains("sk-ant-[REDACTED]"));
    }

    #[test]
    fn test_redacts_openai_api_key() {
        let fixture = "sk-XXXXXXXXXXXXXXXXXXXXXXXXXXXX";
        let pasted = format!("API key: {fixture}");
        let safe = redact_sensitive(&pasted);
        assert!(!safe.contains(fixture));
        assert!(safe.contains("sk-[REDACTED]"));
    }

    #[test]
    fn test_redacts_slack_token() {
        // Test fixture is intentionally clearly-fake (X-padded) so GitHub's
        // push-protection secret scanner doesn't flag it as a real token.
        let pasted = "Slack: xoxb-XXXXXXXXXX-XXXXXXXXXXXXXXXX";
        let safe = redact_sensitive(pasted);
        assert!(!safe.contains("xoxb-XXXXXXXXXX-XXXXXXXXXXXXXXXX"));
        assert!(safe.contains("xox-[REDACTED]"));
    }

    #[test]
    fn test_redacts_github_pat() {
        let fixture = "ghp_XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX";
        let pasted = format!("GH token {fixture}");
        let safe = redact_sensitive(&pasted);
        assert!(!safe.contains(fixture));
        assert!(safe.contains("ghp_[REDACTED]"));
    }

    #[test]
    fn test_redacts_aws_access_key() {
        let fixture = "AKIAXXXXXXXXXXXXXXXX";
        let pasted = format!("AWS_ACCESS_KEY={fixture}");
        let safe = redact_sensitive(&pasted);
        assert!(!safe.contains(fixture));
        assert!(safe.contains("AKIA[REDACTED]"));
    }

    #[test]
    fn test_redacts_google_api_key() {
        let fixture = "AIzaXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX";
        let pasted = format!("Google API: {fixture}");
        let safe = redact_sensitive(&pasted);
        assert!(!safe.contains(fixture));
        assert!(safe.contains("AIza[REDACTED]"));
    }

    #[test]
    fn test_redacts_bearer_token() {
        let header = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.thisIsATokenPayload.signature";
        let safe = redact_sensitive(header);
        assert!(!safe.contains("eyJhbGciOiJIUzI1NiJ9.thisIsATokenPayload.signature"));
        assert!(safe.to_lowercase().contains("bearer [redacted]"));
    }

    #[test]
    fn test_redacts_password_assignment() {
        let snippet = "password=hunter2_my_secret";
        let safe = redact_sensitive(snippet);
        assert!(!safe.contains("hunter2_my_secret"));
        assert!(safe.to_lowercase().contains("password=[redacted]"));
    }

    #[test]
    fn test_redacts_private_key_block() {
        let key = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEAabcdefghijklmnopqrst\n-----END RSA PRIVATE KEY-----";
        let safe = redact_sensitive(key);
        assert!(!safe.contains("MIIEpAIBAAKCAQEAabcdefghijklmnopqrst"));
        assert!(safe.contains("[REDACTED PRIVATE KEY]"));
    }

    /// Functional: ordinary content passes through unchanged.
    #[test]
    fn test_passes_through_normal_text() {
        let normal = "Run cargo test --lib to run all tests.\nThe test count is 1200.";
        assert_eq!(redact_sensitive(normal), normal);
    }

    /// Functional: emails are NOT redacted (legitimately appear in code).
    #[test]
    fn test_does_not_redact_emails() {
        let snippet = "Contact: dev@example.com or maintainer@anthropic.com";
        assert_eq!(redact_sensitive(snippet), snippet);
    }

    /// Idempotency: running twice gives the same result as once.
    #[test]
    fn test_idempotent() {
        let pasted = "key=sk-ant-XXXXXXXXXXXXXXXXXXXXXXXXXXXX";
        let once = redact_sensitive(pasted);
        let twice = redact_sensitive(&once);
        assert_eq!(once, twice);
    }

    /// SECURITY (CAT 3): A short fake "sk-" prefix that's NOT a real
    /// credential (e.g. `sk-test`) shouldn't be redacted — the 20-char
    /// tail requirement should prevent over-aggressive redaction.
    #[test]
    fn test_does_not_redact_short_sk_prefix() {
        let normal = "branch sk-test passed";
        assert_eq!(redact_sensitive(normal), normal);
    }
}
