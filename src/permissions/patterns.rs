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
///
/// **macOS canonical equivalents are listed alongside the surface paths.** On
/// macOS, `/etc`, `/var`, `/tmp`, etc. are symlinks to `/private/etc`,
/// `/private/var`, `/private/tmp`. After path canonicalization
/// (`path_validator::canonical_match_form`), a path like `/etc/passwd`
/// resolves to `/private/etc/passwd`. Both prefixes are listed so the
/// blocklist matches whichever form the canonicalizer produced.
pub const HARD_BLOCKED_PATH_PREFIXES: &[&str] = &[
    // Unix/macOS surface paths
    "/etc/",
    "/system/",
    "/library/",
    "/usr/",
    "/var/",
    "/bin/",
    "/sbin/",
    // macOS canonical equivalents (symlink targets in /private)
    "/private/etc/",
    "/private/var/",
    "/private/system/",
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

/// Additional paths blocked specifically for READS. These are files that hold
/// secrets but live under directories where many other files are legitimately
/// readable (so we can't blanket-block the whole directory). Checked as
/// `contains` patterns against the canonicalized lowercased path.
///
/// Intentionally narrow: directory-level credential blocklists are already
/// in `SENSITIVE_PATH_PATTERNS` (`.ssh/`, `keychains/`, `.gnupg/`,
/// `.aws/credentials`). This list only adds *specific files* that don't
/// live under one of those directories.
///
/// SECURITY (CAT 2 — Path & File Security):
/// Without this list, `file_read` of `/etc/shadow` succeeds (the dir
/// `/etc/` is full of legitimately-readable config files like `/etc/hosts`,
/// so we can't blocklist the whole directory). AUDIT-forge-2026-04-28.md P0 #7.
///
/// Generic credential filenames like `id_rsa` are NOT in this list because
/// `.ssh/` already covers them via `SENSITIVE_PATH_PATTERNS`. Listing
/// `id_rsa` standalone would false-positive on legitimate file names that
/// happen to contain that substring.
pub const READ_BLOCKED_PATH_FRAGMENTS: &[&str] = &[
    "/etc/shadow",
    "/etc/sudoers",
    "/etc/master.passwd",   // BSD/macOS shadow equivalent
    "/private/etc/shadow",  // macOS canonical
    "/private/etc/sudoers", // macOS canonical
    "/private/etc/master.passwd",
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
