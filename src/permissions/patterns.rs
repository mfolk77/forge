/// Compile-time constant arrays for permission classification.
/// These are NOT user-configurable — they are safety invariants.

/// Commands that are always hard-blocked, no override possible.
/// FolkTech Coding Rules v1.3 Section 13.3 deny list.
pub const HARD_BLOCKED_COMMANDS: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    "rm -rf ~",
    "rm -rf ~/",
    "sudo rm -rf /",
    "sudo rm -rf /*",
    "git push --force origin main",
    "git push --force origin master",
    "git reset --hard",
];

/// Path prefixes that are always hard-blocked for writes.
/// Covers FolkTech Coding Rules v1.3 CAT 2 + CAT 6 blocklist.
/// All paths use forward slashes — classifier normalizes backslashes before matching.
pub const HARD_BLOCKED_PATH_PREFIXES: &[&str] = &[
    // Unix/macOS
    "/etc/",
    "/system/",
    "/library/",
    "/usr/",
    "/var/",
    "/bin/",
    "/sbin/",
    // Windows (lowercased, forward-slash normalized)
    "c:/windows/",
    "c:/program files/",
    "c:/program files (x86)/",
    "c:/programdata/",
];

/// Patterns indicating fork bombs or similar — checked via contains.
pub const HARD_BLOCKED_PATTERNS: &[&str] = &[
    ":(){ :|:& };:",
    "fork()",
    ".fork()",
];

/// Command prefixes that indicate destructive operations (Unix + Windows).
pub const DESTRUCTIVE_BASH_PATTERNS: &[&str] = &[
    // Unix
    "rm ",
    "kill ",
    "chmod ",
    "sudo ",
    "mkfs",
    "dd ",
    // Windows (case-insensitive matching handled by classifier)
    "format ",
    "rd /s",
    "rmdir /s",
    "del /f",
    "del /s",
];

/// Path prefixes that indicate system-level file access (destructive for writes).
/// Forward-slash normalized, lowercased for matching.
pub const SYSTEM_PATH_PREFIXES: &[&str] = &[
    // Unix/macOS
    "/etc/",
    "/system/",
    "/usr/",
    "/var/",
    "/library/",
    "/bin/",
    "/sbin/",
    // Windows
    "c:/windows/",
    "c:/program files/",
    "c:/program files (x86)/",
];

/// Sensitive user directory patterns — blocked for writes (CAT 2 + CAT 6).
/// Checked as contains patterns. Forward-slash normalized for cross-platform.
pub const SENSITIVE_PATH_PATTERNS: &[&str] = &[
    ".ssh/",
    "library/keychains/",
    ".gnupg/",
    ".aws/credentials",
    // Windows credential stores
    "appdata/roaming/microsoft/credentials/",
    "appdata/roaming/microsoft/protect/",
];

/// Windows reserved device names (CAT 6).
/// These cannot be used as filenames on Windows and can cause undefined behavior.
pub const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL",
    "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Commands that are always safe (read-only).
pub const SAFE_BASH_COMMANDS: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "grep",
    "rg",
    "find",
    "which",
    "whoami",
    "pwd",
    "echo",
    "wc",
    "diff",
    "file",
    "stat",
    "du",
    "df",
    "uname",
    "date",
    "env",
    "printenv",
    "cargo test",
    "cargo check",
    "cargo clippy",
    "cargo fmt --check",
    "cargo build",
    "npm test",
    "npm run",
    "npx tsc",
    "git status",
    "git diff",
    "git log",
    "git show",
    "git branch",
    "git remote",
    "git tag",
    "swift test",
    "swift build",
    "xcodebuild",
];

/// Bash commands classified as Write (build/install, not destructive).
pub const WRITE_BASH_COMMANDS: &[&str] = &[
    "cargo build",
    "cargo install",
    "npm install",
    "npm ci",
    "pip install",
    "brew install",
    "apt install",
    "apt-get install",
    "mkdir",
    "touch",
    "cp ",
    "mv ",
    "tee ",
];
