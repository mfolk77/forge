/// Compile-time constant arrays for permission classification.
/// These are NOT user-configurable — they are safety invariants.

/// Commands that are always hard-blocked, no override possible.
pub const HARD_BLOCKED_COMMANDS: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    "rm -rf ~",
    "rm -rf ~/",
];

/// Paths that are always hard-blocked for writes.
pub const HARD_BLOCKED_PATHS: &[&str] = &[
    "/etc/passwd",
    "/etc/shadow",
];

/// Path prefixes that are always hard-blocked for writes.
pub const HARD_BLOCKED_PATH_PREFIXES: &[&str] = &[
    "/System",
    "/Library",
];

/// Patterns indicating fork bombs or similar — checked via contains.
pub const HARD_BLOCKED_PATTERNS: &[&str] = &[
    ":(){ :|:& };:",
    "fork()",
    ".fork()",
];

/// Bash command prefixes that indicate destructive operations.
pub const DESTRUCTIVE_BASH_PATTERNS: &[&str] = &[
    "rm ",
    "kill ",
    "chmod ",
    "sudo ",
    "mkfs",
    "dd ",
    "format ",
];

/// Path prefixes that indicate system-level file access (destructive for writes).
pub const SYSTEM_PATH_PREFIXES: &[&str] = &[
    "/etc/",
    "/System/",
    "/usr/",
    "/var/",
    "/Library/",
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
