pub mod indexer;
pub mod query;
pub mod store;
pub mod watcher;

#[cfg(test)]
mod security_tests;

pub use indexer::{ChunkType, CodeChunk, CodeIndexer, IndexProgress};
pub use query::{SearchEngine, SearchResult};
pub use store::{SearchStore, StoredChunk};
pub use watcher::FileWatcher;

/// Known code file extensions worth indexing.
pub(crate) const CODE_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "c", "cpp", "h", "hpp", "java", "kt", "swift",
    "rb", "lua", "zig", "hs", "ml", "mli", "ex", "exs", "erl", "hrl", "cs", "fs", "scala",
    "clj", "cljs", "el", "vim", "sh", "bash", "zsh", "fish", "toml", "yaml", "yml", "json",
    "xml", "html", "css", "scss", "sass", "sql", "proto", "graphql", "md", "txt", "dockerfile",
    "makefile",
];

/// Directories that should always be skipped during indexing.
pub(crate) const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "build",
    "dist",
    ".forge",
    ".ftai",
];

/// Check whether a file extension belongs to a code file we should index.
pub(crate) fn is_code_file(path: &std::path::Path) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_lowercase(),
        None => {
            // Handle extensionless files by name (Makefile, Dockerfile, etc.)
            return path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| {
                    let lower = n.to_lowercase();
                    lower == "makefile" || lower == "dockerfile" || lower == "rakefile"
                })
                .unwrap_or(false);
        }
    };
    CODE_EXTENSIONS.contains(&ext.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_is_code_file_rust() {
        assert!(is_code_file(Path::new("src/main.rs")));
        assert!(is_code_file(Path::new("lib.rs")));
    }

    #[test]
    fn test_is_code_file_python() {
        assert!(is_code_file(Path::new("app.py")));
    }

    #[test]
    fn test_is_code_file_typescript() {
        assert!(is_code_file(Path::new("component.tsx")));
        assert!(is_code_file(Path::new("index.ts")));
    }

    #[test]
    fn test_is_code_file_rejects_binaries() {
        assert!(!is_code_file(Path::new("image.png")));
        assert!(!is_code_file(Path::new("archive.zip")));
        assert!(!is_code_file(Path::new("binary.exe")));
        assert!(!is_code_file(Path::new("library.so")));
    }

    #[test]
    fn test_is_code_file_extensionless() {
        assert!(is_code_file(Path::new("Makefile")));
        assert!(is_code_file(Path::new("Dockerfile")));
        assert!(!is_code_file(Path::new("README")));
    }

    #[test]
    fn test_is_code_file_case_insensitive_ext() {
        assert!(is_code_file(Path::new("Module.RS")));
        assert!(is_code_file(Path::new("App.PY")));
    }

    #[test]
    fn test_skip_dirs_contains_expected() {
        assert!(SKIP_DIRS.contains(&".git"));
        assert!(SKIP_DIRS.contains(&"node_modules"));
        assert!(SKIP_DIRS.contains(&"target"));
        assert!(SKIP_DIRS.contains(&".ftai"));
    }
}
