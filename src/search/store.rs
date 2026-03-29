use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

/// A chunk record retrieved from the database.
#[derive(Debug, Clone)]
pub struct StoredChunk {
    pub id: i64,
    pub file_path: String,
    pub chunk_type: String,
    pub symbol_name: Option<String>,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    pub embedding: Vec<f32>,
    pub file_mtime: f64,
    pub indexed_at: f64,
}

/// SQLite-backed vector store for code chunk embeddings.
#[derive(Debug)]
pub struct SearchStore {
    conn: Connection,
}

impl SearchStore {
    /// Open (or create) the search database at `db_path`. Uses WAL mode for
    /// concurrent reads during indexing.
    pub fn open(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("failed to open search db at {}", db_path.display()))?;

        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chunks (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path   TEXT    NOT NULL,
                chunk_type  TEXT    NOT NULL,
                symbol_name TEXT,
                start_line  INTEGER NOT NULL,
                end_line    INTEGER NOT NULL,
                content     TEXT    NOT NULL,
                embedding   BLOB    NOT NULL,
                file_mtime  REAL    NOT NULL,
                indexed_at  REAL    NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_chunks_file_path ON chunks(file_path);
            CREATE INDEX IF NOT EXISTS idx_chunks_symbol    ON chunks(symbol_name);",
        )?;

        Ok(Self { conn })
    }

    /// Delete all chunks for `file_path`, then insert the new set.
    ///
    /// `chunks` is a slice of `(chunk_type, symbol_name, start_line, end_line, content, embedding)`.
    pub fn upsert_chunks(
        &self,
        file_path: &str,
        chunks: &[(&str, Option<&str>, u32, u32, &str, &[f32])],
        mtime: f64,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM chunks WHERE file_path = ?1", params![file_path])?;

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO chunks (file_path, chunk_type, symbol_name, start_line, end_line, content, embedding, file_mtime, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;

            for (chunk_type, symbol_name, start_line, end_line, content, embedding) in chunks {
                let blob = embedding_to_blob(embedding);
                stmt.execute(params![
                    file_path,
                    chunk_type,
                    symbol_name,
                    start_line,
                    end_line,
                    content,
                    blob,
                    mtime,
                    now,
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Remove all chunks belonging to `file_path`.
    pub fn delete_file(&self, file_path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM chunks WHERE file_path = ?1", params![file_path])?;
        Ok(())
    }

    /// Load every (id, embedding) pair in the database. Used for brute-force
    /// nearest-neighbour search.
    pub fn all_embeddings(&self) -> Result<(Vec<i64>, Vec<Vec<f32>>)> {
        let mut stmt = self.conn.prepare("SELECT id, embedding FROM chunks")?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((id, blob))
        })?;

        let mut ids = Vec::new();
        let mut embeddings = Vec::new();
        for row in rows {
            let (id, blob) = row?;
            ids.push(id);
            embeddings.push(blob_to_embedding(&blob));
        }
        Ok((ids, embeddings))
    }

    /// Retrieve a single chunk by its primary key.
    pub fn get_chunk(&self, id: i64) -> Result<StoredChunk> {
        self.conn
            .query_row(
                "SELECT id, file_path, chunk_type, symbol_name, start_line, end_line,
                        content, embedding, file_mtime, indexed_at
                 FROM chunks WHERE id = ?1",
                params![id],
                |row| {
                    let blob: Vec<u8> = row.get(7)?;
                    Ok(StoredChunk {
                        id: row.get(0)?,
                        file_path: row.get(1)?,
                        chunk_type: row.get(2)?,
                        symbol_name: row.get(3)?,
                        start_line: row.get(4)?,
                        end_line: row.get(5)?,
                        content: row.get(6)?,
                        embedding: blob_to_embedding(&blob),
                        file_mtime: row.get(8)?,
                        indexed_at: row.get(9)?,
                    })
                },
            )
            .with_context(|| format!("chunk id={id} not found"))
    }

    /// Returns `true` if the file is already indexed with the given mtime,
    /// meaning it does not need re-indexing.
    pub fn is_current(&self, file_path: &str, mtime: f64) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM chunks WHERE file_path = ?1 AND file_mtime = ?2 LIMIT 1",
                params![file_path, mtime],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// Remove chunks for files that no longer exist on disk relative to `root`.
    pub fn prune_deleted_files(&self, root: &Path) -> Result<usize> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT file_path FROM chunks")?;
        let paths: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut pruned = 0usize;
        for file_path in &paths {
            let full = root.join(file_path);
            if !full.exists() {
                self.delete_file(file_path)?;
                pruned += 1;
            }
        }
        Ok(pruned)
    }

    /// Return the total number of indexed chunks.
    pub fn chunk_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))?;
        Ok(count as usize)
    }
}

// ---------------------------------------------------------------------------
// Embedding serialization helpers
// ---------------------------------------------------------------------------

/// Serialize `&[f32]` to a little-endian byte blob.
fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        buf.extend_from_slice(&val.to_le_bytes());
    }
    buf
}

/// Deserialize a little-endian byte blob back to `Vec<f32>`.
fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().expect("chunks_exact guarantees 4 bytes");
            f32::from_le_bytes(arr)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_store() -> (TempDir, SearchStore) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test_search.db");
        let store = SearchStore::open(&db_path).unwrap();
        (dir, store)
    }

    #[test]
    fn test_open_creates_db() {
        let (_dir, store) = temp_store();
        assert_eq!(store.chunk_count().unwrap(), 0);
    }

    #[test]
    fn test_upsert_and_get_chunk() {
        let (_dir, store) = temp_store();
        let emb = vec![0.1_f32, 0.2, 0.3];
        store
            .upsert_chunks(
                "src/main.rs",
                &[("Function", Some("main"), 1, 10, "fn main() {}", emb.as_slice())],
                1000.0,
            )
            .unwrap();

        assert_eq!(store.chunk_count().unwrap(), 1);

        let chunk = store.get_chunk(1).unwrap();
        assert_eq!(chunk.file_path, "src/main.rs");
        assert_eq!(chunk.chunk_type, "Function");
        assert_eq!(chunk.symbol_name.as_deref(), Some("main"));
        assert_eq!(chunk.start_line, 1);
        assert_eq!(chunk.end_line, 10);
        assert_eq!(chunk.content, "fn main() {}");
        assert_eq!(chunk.embedding.len(), 3);
        assert!((chunk.embedding[0] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn test_upsert_replaces_old_chunks() {
        let (_dir, store) = temp_store();
        let emb = vec![1.0_f32; 4];
        store
            .upsert_chunks(
                "src/lib.rs",
                &[
                    ("Function", Some("foo"), 1, 5, "fn foo() {}", &emb),
                    ("Function", Some("bar"), 6, 10, "fn bar() {}", &emb),
                ],
                100.0,
            )
            .unwrap();
        assert_eq!(store.chunk_count().unwrap(), 2);

        // Re-index with a single chunk should replace both.
        store
            .upsert_chunks(
                "src/lib.rs",
                &[("Function", Some("baz"), 1, 15, "fn baz() {}", &emb)],
                200.0,
            )
            .unwrap();
        assert_eq!(store.chunk_count().unwrap(), 1);
    }

    #[test]
    fn test_is_current() {
        let (_dir, store) = temp_store();
        let emb = vec![0.0_f32; 2];
        store
            .upsert_chunks("a.rs", &[("Block", None, 1, 40, "code", &emb)], 500.0)
            .unwrap();

        assert!(store.is_current("a.rs", 500.0));
        assert!(!store.is_current("a.rs", 501.0));
        assert!(!store.is_current("b.rs", 500.0));
    }

    #[test]
    fn test_delete_file() {
        let (_dir, store) = temp_store();
        let emb = vec![0.0_f32; 2];
        store
            .upsert_chunks("a.rs", &[("Block", None, 1, 10, "x", &emb)], 1.0)
            .unwrap();
        store
            .upsert_chunks("b.rs", &[("Block", None, 1, 10, "y", &emb)], 1.0)
            .unwrap();
        assert_eq!(store.chunk_count().unwrap(), 2);

        store.delete_file("a.rs").unwrap();
        assert_eq!(store.chunk_count().unwrap(), 1);
    }

    #[test]
    fn test_all_embeddings() {
        let (_dir, store) = temp_store();
        let emb1 = vec![1.0_f32, 2.0, 3.0];
        let emb2 = vec![4.0_f32, 5.0, 6.0];
        store
            .upsert_chunks("a.rs", &[("Function", Some("a"), 1, 5, "fn a()", &emb1)], 1.0)
            .unwrap();
        store
            .upsert_chunks("b.rs", &[("Function", Some("b"), 1, 5, "fn b()", &emb2)], 1.0)
            .unwrap();

        let (ids, embeddings) = store.all_embeddings().unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].len(), 3);
    }

    #[test]
    fn test_prune_deleted_files() {
        let (dir, store) = temp_store();
        let root = dir.path();
        // Create one real file and leave one as ghost.
        std::fs::write(root.join("real.rs"), "fn real() {}").unwrap();

        let emb = vec![0.0_f32; 2];
        store
            .upsert_chunks("real.rs", &[("Block", None, 1, 1, "x", &emb)], 1.0)
            .unwrap();
        store
            .upsert_chunks("ghost.rs", &[("Block", None, 1, 1, "y", &emb)], 1.0)
            .unwrap();
        assert_eq!(store.chunk_count().unwrap(), 2);

        let pruned = store.prune_deleted_files(root).unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(store.chunk_count().unwrap(), 1);
    }

    #[test]
    fn test_embedding_round_trip() {
        let original: Vec<f32> = vec![1.0, -1.0, 0.0, 3.14, f32::MIN, f32::MAX];
        let blob = embedding_to_blob(&original);
        let restored = blob_to_embedding(&blob);
        assert_eq!(original, restored);
    }

    #[test]
    fn test_blob_to_embedding_empty() {
        let restored = blob_to_embedding(&[]);
        assert!(restored.is_empty());
    }
}
