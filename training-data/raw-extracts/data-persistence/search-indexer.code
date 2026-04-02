use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use ignore::WalkBuilder;

use super::store::SearchStore;
use super::{is_code_file, SKIP_DIRS};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The kind of code construct a chunk represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ChunkType {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Impl,
    Trait,
    Module,
    Block,
}

impl ChunkType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "Function",
            Self::Method => "Method",
            Self::Class => "Class",
            Self::Struct => "Struct",
            Self::Enum => "Enum",
            Self::Impl => "Impl",
            Self::Trait => "Trait",
            Self::Module => "Module",
            Self::Block => "Block",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "Function" => Self::Function,
            "Method" => Self::Method,
            "Class" => Self::Class,
            "Struct" => Self::Struct,
            "Enum" => Self::Enum,
            "Impl" => Self::Impl,
            "Trait" => Self::Trait,
            "Module" => Self::Module,
            _ => Self::Block,
        }
    }
}

impl std::fmt::Display for ChunkType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A code chunk extracted from a source file.
#[derive(Debug, Clone)]
pub struct CodeChunk {
    pub file_path: String,
    pub chunk_type: ChunkType,
    pub symbol_name: Option<String>,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
}

/// Progress information emitted during indexing.
#[derive(Debug, Clone)]
pub struct IndexProgress {
    pub files_total: usize,
    pub files_done: usize,
    pub chunks_so_far: usize,
    pub current_file: PathBuf,
}

// ---------------------------------------------------------------------------
// CodeIndexer
// ---------------------------------------------------------------------------

/// Main indexer: walks a project, chunks source files, generates embeddings,
/// and writes them to the vector store.
pub struct CodeIndexer {
    embedder: TextEmbedding,
    store: SearchStore,
    project_root: PathBuf,
}

impl std::fmt::Debug for CodeIndexer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeIndexer")
            .field("store", &self.store)
            .field("project_root", &self.project_root)
            .finish()
    }
}

impl CodeIndexer {
    /// Create a new indexer backed by the SQLite database at `db_path`.
    ///
    /// Initialises fastembed with the BGE-small-en-v1.5 model.
    pub fn new(db_path: &Path) -> Result<Self> {
        let store = SearchStore::open(db_path)?;
        let embedder = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15).with_show_download_progress(false),
        )
        .context("failed to initialise fastembed BGE-small-en-v1.5")?;

        Ok(Self {
            embedder,
            store,
            project_root: PathBuf::new(),
        })
    }

    /// Index an entire project tree. Calls `progress` after each file.
    pub fn index_project(
        &mut self,
        root: &Path,
        progress: impl Fn(IndexProgress),
    ) -> Result<()> {
        self.project_root = root.to_path_buf();

        let files = collect_files(root)?;
        let total = files.len();
        let mut chunks_total = 0usize;

        for (i, file_path) in files.iter().enumerate() {
            let rel = file_path
                .strip_prefix(root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            // Check mtime to skip unchanged files.
            let mtime = file_mtime(file_path);
            if self.store.is_current(&rel, mtime) {
                progress(IndexProgress {
                    files_total: total,
                    files_done: i + 1,
                    chunks_so_far: chunks_total,
                    current_file: file_path.clone(),
                });
                continue;
            }

            match self.index_single_file(file_path, &rel, mtime) {
                Ok(n) => chunks_total += n,
                Err(e) => {
                    // Log but do not abort the whole project for a single file.
                    eprintln!("warn: failed to index {rel}: {e}");
                }
            }

            progress(IndexProgress {
                files_total: total,
                files_done: i + 1,
                chunks_so_far: chunks_total,
                current_file: file_path.clone(),
            });
        }

        // Clean up chunks for deleted files.
        let _ = self.store.prune_deleted_files(root);

        Ok(())
    }

    /// Re-index a single file (incremental update).
    pub fn update_file(&mut self, file_path: &Path) -> Result<()> {
        let rel = file_path
            .strip_prefix(&self.project_root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        if !file_path.exists() {
            self.store.delete_file(&rel)?;
            return Ok(());
        }

        let mtime = file_mtime(file_path);
        self.index_single_file(file_path, &rel, mtime)?;
        Ok(())
    }

    /// Returns a reference to the underlying store.
    pub fn store(&self) -> &SearchStore {
        &self.store
    }

    /// Embed a single query string using the same model used for indexing.
    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let results = self
            .embedder
            .embed(vec![query], None)
            .context("failed to embed query")?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("embedding model returned no vectors"))
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    fn index_single_file(
        &self,
        file_path: &Path,
        rel: &str,
        mtime: f64,
    ) -> Result<usize> {
        let source = fs::read_to_string(file_path)
            .with_context(|| format!("reading {}", file_path.display()))?;

        let chunks = chunk_source(rel, &source);
        if chunks.is_empty() {
            // Still mark the file as indexed so we don't retry.
            self.store.upsert_chunks(rel, &[], mtime)?;
            return Ok(0);
        }

        let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        let embeddings = self
            .embedder
            .embed(texts, None)
            .context("embedding generation failed")?;

        let rows: Vec<(&str, Option<&str>, u32, u32, &str, &[f32])> = chunks
            .iter()
            .zip(embeddings.iter())
            .map(|(c, emb)| {
                (
                    c.chunk_type.as_str(),
                    c.symbol_name.as_deref(),
                    c.start_line,
                    c.end_line,
                    c.content.as_str(),
                    emb.as_slice(),
                )
            })
            .collect();

        self.store.upsert_chunks(rel, &rows, mtime)?;
        Ok(chunks.len())
    }
}

// ---------------------------------------------------------------------------
// File collection
// ---------------------------------------------------------------------------

/// Walk the project using the `ignore` crate (respects .gitignore) and return
/// all code files worth indexing.
fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    let walker = WalkBuilder::new(root)
        .hidden(true) // skip hidden by default
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| {
            if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                let name = entry.file_name().to_string_lossy();
                return !SKIP_DIRS.contains(&name.as_ref());
            }
            true
        })
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().map_or(true, |ft| !ft.is_file()) {
            continue;
        }
        let path = entry.into_path();
        if is_code_file(&path) {
            files.push(path);
        }
    }

    Ok(files)
}

/// Get the modification time of a file as seconds since UNIX epoch.
fn file_mtime(path: &Path) -> f64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

/// Heuristic patterns that mark the start of a named code construct.
struct BoundaryPattern {
    keyword: &'static str,
    chunk_type: ChunkType,
}

const BOUNDARY_PATTERNS: &[BoundaryPattern] = &[
    BoundaryPattern { keyword: "fn ", chunk_type: ChunkType::Function },
    BoundaryPattern { keyword: "pub fn ", chunk_type: ChunkType::Function },
    BoundaryPattern { keyword: "pub(crate) fn ", chunk_type: ChunkType::Function },
    BoundaryPattern { keyword: "async fn ", chunk_type: ChunkType::Function },
    BoundaryPattern { keyword: "pub async fn ", chunk_type: ChunkType::Function },
    BoundaryPattern { keyword: "def ", chunk_type: ChunkType::Function },
    BoundaryPattern { keyword: "async def ", chunk_type: ChunkType::Function },
    BoundaryPattern { keyword: "func ", chunk_type: ChunkType::Function },
    BoundaryPattern { keyword: "function ", chunk_type: ChunkType::Function },
    BoundaryPattern { keyword: "class ", chunk_type: ChunkType::Class },
    BoundaryPattern { keyword: "pub class ", chunk_type: ChunkType::Class },
    BoundaryPattern { keyword: "struct ", chunk_type: ChunkType::Struct },
    BoundaryPattern { keyword: "pub struct ", chunk_type: ChunkType::Struct },
    BoundaryPattern { keyword: "pub(crate) struct ", chunk_type: ChunkType::Struct },
    BoundaryPattern { keyword: "enum ", chunk_type: ChunkType::Enum },
    BoundaryPattern { keyword: "pub enum ", chunk_type: ChunkType::Enum },
    BoundaryPattern { keyword: "pub(crate) enum ", chunk_type: ChunkType::Enum },
    BoundaryPattern { keyword: "impl ", chunk_type: ChunkType::Impl },
    BoundaryPattern { keyword: "trait ", chunk_type: ChunkType::Trait },
    BoundaryPattern { keyword: "pub trait ", chunk_type: ChunkType::Trait },
    BoundaryPattern { keyword: "pub(crate) trait ", chunk_type: ChunkType::Trait },
    BoundaryPattern { keyword: "mod ", chunk_type: ChunkType::Module },
    BoundaryPattern { keyword: "pub mod ", chunk_type: ChunkType::Module },
];

/// Detected boundary position in source text.
#[derive(Debug)]
struct Boundary {
    line_idx: usize,
    chunk_type: ChunkType,
    symbol_name: Option<String>,
}

/// Chunk source code into semantically meaningful pieces.
///
/// Uses line-based heuristics to find function/struct/class boundaries. Falls
/// back to a sliding window for files where no boundaries are detected.
pub fn chunk_source(file_path: &str, source: &str) -> Vec<CodeChunk> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let boundaries = detect_boundaries(&lines);

    if boundaries.is_empty() {
        return sliding_window_chunks(file_path, &lines);
    }

    boundary_chunks(file_path, &lines, &boundaries)
}

/// Find lines that look like the start of a named construct.
fn detect_boundaries(lines: &[&str]) -> Vec<Boundary> {
    let mut boundaries = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        // Also match lines starting with decorators/attributes — we skip those
        // and look for the next real keyword.
        for pat in BOUNDARY_PATTERNS {
            if trimmed.starts_with(pat.keyword) {
                let name = extract_symbol_name(trimmed, pat.keyword);
                boundaries.push(Boundary {
                    line_idx: idx,
                    chunk_type: pat.chunk_type,
                    symbol_name: name,
                });
                break;
            }
        }
    }

    boundaries
}

/// Pull out the symbol name after the keyword.
/// e.g. `"fn main() {"` with keyword `"fn "` -> `"main"`.
fn extract_symbol_name(trimmed: &str, keyword: &str) -> Option<String> {
    let after = &trimmed[keyword.len()..];
    let name: String = after
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Convert detected boundaries into chunks. Each chunk runs from its boundary
/// line until the line before the next boundary (or EOF).
fn boundary_chunks(file_path: &str, lines: &[&str], boundaries: &[Boundary]) -> Vec<CodeChunk> {
    let mut chunks = Vec::new();

    // If the file starts with content before the first boundary, capture it.
    if boundaries[0].line_idx > 0 {
        let content = lines[..boundaries[0].line_idx].join("\n");
        if !content.trim().is_empty() {
            chunks.push(CodeChunk {
                file_path: file_path.to_string(),
                chunk_type: ChunkType::Block,
                symbol_name: None,
                start_line: 1,
                end_line: boundaries[0].line_idx as u32,
                content,
            });
        }
    }

    for (i, boundary) in boundaries.iter().enumerate() {
        let start = boundary.line_idx;
        let end = if i + 1 < boundaries.len() {
            boundaries[i + 1].line_idx
        } else {
            lines.len()
        };

        let content = lines[start..end].join("\n");
        if content.trim().is_empty() {
            continue;
        }

        chunks.push(CodeChunk {
            file_path: file_path.to_string(),
            chunk_type: boundary.chunk_type,
            symbol_name: boundary.symbol_name.clone(),
            start_line: (start + 1) as u32,
            end_line: end as u32,
            content,
        });
    }

    chunks
}

/// Fallback: 40-line windows with 10-line overlap.
fn sliding_window_chunks(file_path: &str, lines: &[&str]) -> Vec<CodeChunk> {
    const WINDOW: usize = 40;
    const OVERLAP: usize = 10;
    let step = WINDOW.saturating_sub(OVERLAP).max(1);

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < lines.len() {
        let end = (start + WINDOW).min(lines.len());
        let content = lines[start..end].join("\n");
        if !content.trim().is_empty() {
            chunks.push(CodeChunk {
                file_path: file_path.to_string(),
                chunk_type: ChunkType::Block,
                symbol_name: None,
                start_line: (start + 1) as u32,
                end_line: end as u32,
                content,
            });
        }
        if end == lines.len() {
            break;
        }
        start += step;
    }

    chunks
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_rust_functions() {
        let source = r#"use std::io;

fn foo() {
    println!("foo");
}

pub fn bar(x: i32) -> i32 {
    x + 1
}
"#;
        let chunks = chunk_source("test.rs", source);
        assert!(chunks.len() >= 2, "expected at least 2 chunks, got {}", chunks.len());

        let fn_names: Vec<Option<&str>> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Function)
            .map(|c| c.symbol_name.as_deref())
            .collect();
        assert!(fn_names.contains(&Some("foo")));
        assert!(fn_names.contains(&Some("bar")));
    }

    #[test]
    fn test_chunk_python_functions() {
        let source = "import os\n\ndef hello():\n    print('hi')\n\nclass MyClass:\n    pass\n";
        let chunks = chunk_source("app.py", source);
        let types: Vec<ChunkType> = chunks.iter().map(|c| c.chunk_type).collect();
        assert!(types.contains(&ChunkType::Function));
        assert!(types.contains(&ChunkType::Class));
    }

    #[test]
    fn test_chunk_struct_and_impl() {
        let source = "pub struct Foo {\n    x: i32,\n}\n\nimpl Foo {\n    fn new() -> Self { Self { x: 0 } }\n}\n";
        let chunks = chunk_source("foo.rs", source);
        let types: Vec<ChunkType> = chunks.iter().map(|c| c.chunk_type).collect();
        assert!(types.contains(&ChunkType::Struct));
        assert!(types.contains(&ChunkType::Impl));
    }

    #[test]
    fn test_chunk_empty_file() {
        let chunks = chunk_source("empty.rs", "");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_whitespace_only() {
        let chunks = chunk_source("blank.rs", "   \n\n   \n");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_sliding_window_fallback() {
        // No recognized boundaries — should get sliding window chunks.
        let lines: Vec<String> = (0..100).map(|i| format!("// line {i}")).collect();
        let source = lines.join("\n");
        let chunks = chunk_source("comments.txt", &source);
        assert!(!chunks.is_empty());
        // All chunks should be Block type.
        for c in &chunks {
            assert_eq!(c.chunk_type, ChunkType::Block);
        }
    }

    #[test]
    fn test_sliding_window_overlap() {
        let lines: Vec<String> = (0..60).map(|i| format!("// line {i}")).collect();
        let source = lines.join("\n");
        let chunks = chunk_source("big.txt", &source);
        assert!(chunks.len() >= 2);
        // The second chunk should start before the first chunk ends (overlap).
        if chunks.len() >= 2 {
            assert!(chunks[1].start_line < chunks[0].end_line);
        }
    }

    #[test]
    fn test_extract_symbol_name_simple() {
        assert_eq!(extract_symbol_name("fn main() {", "fn "), Some("main".to_string()));
        assert_eq!(extract_symbol_name("pub fn do_stuff(x: i32)", "pub fn "), Some("do_stuff".to_string()));
        assert_eq!(extract_symbol_name("struct Foo {", "struct "), Some("Foo".to_string()));
        assert_eq!(extract_symbol_name("def __init__(self):", "def "), Some("__init__".to_string()));
    }

    #[test]
    fn test_extract_symbol_name_empty() {
        assert_eq!(extract_symbol_name("fn ()", "fn "), None);
    }

    #[test]
    fn test_chunk_type_display() {
        assert_eq!(ChunkType::Function.as_str(), "Function");
        assert_eq!(ChunkType::Block.as_str(), "Block");
    }

    #[test]
    fn test_chunk_type_round_trip() {
        let variants = [
            ChunkType::Function,
            ChunkType::Method,
            ChunkType::Class,
            ChunkType::Struct,
            ChunkType::Enum,
            ChunkType::Impl,
            ChunkType::Trait,
            ChunkType::Module,
            ChunkType::Block,
        ];
        for v in variants {
            assert_eq!(ChunkType::from_str(v.as_str()), v);
        }
    }

    #[test]
    fn test_chunk_type_from_str_unknown() {
        assert_eq!(ChunkType::from_str("Unknown"), ChunkType::Block);
    }

    #[test]
    fn test_chunk_preserves_line_numbers() {
        let source = "use x;\n\nfn alpha() {\n    1\n}\n\nfn beta() {\n    2\n}\n";
        let chunks = chunk_source("lines.rs", source);
        let alpha = chunks.iter().find(|c| c.symbol_name.as_deref() == Some("alpha"));
        assert!(alpha.is_some());
        let alpha = alpha.unwrap();
        assert_eq!(alpha.start_line, 3);
    }

    #[test]
    fn test_chunk_trait() {
        let source = "pub trait Drawable {\n    fn draw(&self);\n}\n";
        let chunks = chunk_source("traits.rs", source);
        assert!(!chunks.is_empty());
        // First chunk is the trait itself; the inner fn may create a second boundary
        let trait_chunk = chunks.iter().find(|c| c.chunk_type == ChunkType::Trait);
        assert!(trait_chunk.is_some());
        assert_eq!(trait_chunk.unwrap().symbol_name.as_deref(), Some("Drawable"));
    }

    #[test]
    fn test_chunk_enum() {
        let source = "pub enum Color {\n    Red,\n    Green,\n    Blue,\n}\n";
        let chunks = chunk_source("enums.rs", source);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, ChunkType::Enum);
        assert_eq!(chunks[0].symbol_name.as_deref(), Some("Color"));
    }

    #[test]
    fn test_chunk_javascript_function() {
        let source = "const x = 1;\n\nfunction handleClick() {\n    console.log(x);\n}\n";
        let chunks = chunk_source("app.js", source);
        let func = chunks.iter().find(|c| c.chunk_type == ChunkType::Function);
        assert!(func.is_some());
        assert_eq!(func.unwrap().symbol_name.as_deref(), Some("handleClick"));
    }

    #[test]
    fn test_chunk_go_func() {
        let source = "package main\n\nfunc main() {\n    fmt.Println(\"hello\")\n}\n";
        let chunks = chunk_source("main.go", source);
        let func = chunks.iter().find(|c| c.chunk_type == ChunkType::Function);
        assert!(func.is_some());
        assert_eq!(func.unwrap().symbol_name.as_deref(), Some("main"));
    }

    #[test]
    fn test_file_mtime_nonexistent() {
        let mtime = file_mtime(Path::new("/nonexistent/file.rs"));
        assert_eq!(mtime, 0.0);
    }
}
