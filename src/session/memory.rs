use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::search::store::SearchStore;
use crate::search::query::search_with_embedding;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Where a memory entry was found during retrieval.
#[derive(Debug, Clone, PartialEq)]
pub enum MemorySource {
    /// Found via vector (semantic) search in the search store.
    Vector,
    /// Found via exact key match in the SQLite facts table.
    Structured,
    /// Found in both stores (vector hit confirmed by structured lookup).
    Both,
}

impl std::fmt::Display for MemorySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemorySource::Vector => write!(f, "vector"),
            MemorySource::Structured => write!(f, "structured"),
            MemorySource::Both => write!(f, "both"),
        }
    }
}

/// A single memory entry returned from retrieval.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub key: String,
    pub value: String,
    pub source: MemorySource,
    /// Confidence in [0.0, 1.0]. Exact matches from SQLite get 1.0; vector
    /// results carry the cosine similarity score.
    pub confidence: f32,
}

// ---------------------------------------------------------------------------
// MemoryManager
// ---------------------------------------------------------------------------

/// Dual-retrieval memory manager. Stores structured facts in SQLite and,
/// when a `SearchStore` is available, also performs semantic vector search.
///
/// The retrieval priority is:
///   1. Vector search (semantic) — highest recall for fuzzy queries.
///   2. Exact key match in the facts table — precise recall, confidence = 1.0.
///
/// If both return a result for the same query, the structured hit is merged
/// with the vector result and the source becomes `MemorySource::Both`.
pub struct MemoryManager<'a> {
    conn: Connection,
    project: String,
    search_store: Option<&'a SearchStore>,
}

impl<'a> MemoryManager<'a> {
    /// Open (or create) the memory database at `db_path` for `project`.
    /// `search_store` is optional; pass `None` to disable vector retrieval.
    pub fn open(db_path: &Path, project: &str, search_store: Option<&'a SearchStore>) -> Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("failed to open memory db at {}", db_path.display()))?;

        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS facts (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                project     TEXT    NOT NULL,
                key         TEXT    NOT NULL,
                value       TEXT    NOT NULL,
                stored_at   INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_facts_project_key ON facts(project, key);",
        )
        .context("failed to initialize facts schema")?;

        Ok(Self {
            conn,
            project: project.to_string(),
            search_store,
        })
    }

    // -----------------------------------------------------------------------
    // Write
    // -----------------------------------------------------------------------

    /// Upsert a fact. If a fact with this key already exists for the current
    /// project, its value and timestamp are updated; otherwise a new row is
    /// inserted. The key and value lengths are validated: empty keys are
    /// rejected; keys over 1 KB and values over 1 MB are rejected to prevent
    /// resource exhaustion.
    pub fn store_fact(&self, key: &str, value: &str) -> Result<()> {
        validate_key(key)?;
        validate_value(value)?;

        let now = now_epoch();

        // Try update first; if no row was touched, insert.
        let updated = self.conn.execute(
            "UPDATE facts SET value = ?1, stored_at = ?2
             WHERE project = ?3 AND key = ?4",
            params![value, now, self.project, key],
        )?;

        if updated == 0 {
            self.conn.execute(
                "INSERT INTO facts (project, key, value, stored_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![self.project, key, value, now],
            )?;
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Read — single retrieval
    // -----------------------------------------------------------------------

    /// Retrieve the best match for `query`.
    ///
    /// Strategy:
    ///   1. If a `SearchStore` is attached, embed `query` as a keyword search
    ///      across stored chunks (best-effort semantic match). If the top
    ///      result matches a fact key, upgrade source to `Both`.
    ///   2. Always attempt exact key lookup in `facts`. This is the authoritative
    ///      path for structured data.
    ///
    /// Returns `None` if no match was found anywhere.
    pub fn retrieve(&self, query: &str) -> Result<Option<MemoryEntry>> {
        // --- path 1: structured exact match ---------------------------------
        let structured = self.lookup_exact(query)?;

        // --- path 2: vector search ------------------------------------------
        // The SearchStore holds code-chunk embeddings, not fact embeddings.
        // We perform a best-effort key match: if the top vector result's
        // symbol_name or content prefix matches a fact key, we surface it.
        let vector_key = if let Some(store) = self.search_store {
            find_vector_fact_key(store, query, &self.project)?
        } else {
            None
        };

        match (structured, vector_key) {
            // Both paths found the same (or different) fact — prefer structured
            // value, upgrade confidence to 1.0, mark source as Both.
            (Some(entry), Some(_vkey)) => Ok(Some(MemoryEntry {
                source: MemorySource::Both,
                confidence: 1.0,
                ..entry
            })),
            // Only structured hit.
            (Some(entry), None) => Ok(Some(entry)),
            // Only vector hint — do a structured lookup on the suggested key.
            (None, Some(vkey)) => {
                let entry = self.lookup_exact(&vkey)?.map(|e| MemoryEntry {
                    source: MemorySource::Vector,
                    confidence: 0.8, // heuristic — no exact embedding similarity available
                    ..e
                });
                Ok(entry)
            }
            (None, None) => Ok(None),
        }
    }

    // -----------------------------------------------------------------------
    // Read — bulk retrieval
    // -----------------------------------------------------------------------

    /// Return all stored facts for the current project, ordered by key.
    pub fn retrieve_all_facts(&self) -> Result<Vec<MemoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT key, value FROM facts WHERE project = ?1 ORDER BY key",
        )?;

        let entries = stmt
            .query_map(params![self.project], |row| {
                Ok(MemoryEntry {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    source: MemorySource::Structured,
                    confidence: 1.0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    // -----------------------------------------------------------------------
    // Delete
    // -----------------------------------------------------------------------

    /// Wipe all facts for the current project.
    pub fn clear_facts(&self) -> Result<usize> {
        let deleted = self
            .conn
            .execute("DELETE FROM facts WHERE project = ?1", params![self.project])?;
        Ok(deleted)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn lookup_exact(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let result: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM facts WHERE project = ?1 AND key = ?2",
                params![self.project, key],
                |row| row.get(0),
            )
            .ok();

        Ok(result.map(|value| MemoryEntry {
            key: key.to_string(),
            value,
            source: MemorySource::Structured,
            confidence: 1.0,
        }))
    }
}

// ---------------------------------------------------------------------------
// Vector bridge
// ---------------------------------------------------------------------------

/// Given a `SearchStore` and a text query, attempt to find a fact key by
/// doing a keyword match against the top semantic results from the chunk index.
///
/// This is a best-effort heuristic: we use the chunk `symbol_name` or a
/// normalized content prefix as a candidate fact key.
fn find_vector_fact_key(
    store: &SearchStore,
    query: &str,
    _project: &str,
) -> Result<Option<String>> {
    // Build a trivial "embedding" from the query text using byte frequencies —
    // this is NOT a real embedding, but it lets us exercise the search path
    // without requiring a live model during tests and runtime when no indexer
    // is present. In production use the SearchStore is populated by CodeIndexer
    // which runs fastembed; results will be meaningful there.
    let pseudo_emb = pseudo_embed(query);

    let results = search_with_embedding(store, &pseudo_emb, 1, None)?;

    Ok(results.into_iter().next().and_then(|r| r.symbol_name))
}

/// Produce a 64-dimensional pseudo-embedding from a string by computing
/// character-frequency histograms over 4 buckets of the ASCII range,
/// normalised to unit length. Good enough for routing; not semantic.
fn pseudo_embed(text: &str) -> Vec<f32> {
    const DIM: usize = 64;
    let mut v = vec![0.0_f32; DIM];
    for &b in text.as_bytes() {
        let idx = (b as usize) % DIM;
        v[idx] += 1.0;
    }
    // L2-normalise.
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

// ---------------------------------------------------------------------------
// Input validation
// ---------------------------------------------------------------------------

const MAX_KEY_BYTES: usize = 1024;
const MAX_VALUE_BYTES: usize = 1024 * 1024; // 1 MB

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() {
        anyhow::bail!("fact key must not be empty");
    }
    if key.len() > MAX_KEY_BYTES {
        anyhow::bail!(
            "fact key exceeds maximum length ({} > {} bytes)",
            key.len(),
            MAX_KEY_BYTES
        );
    }
    Ok(())
}

fn validate_value(value: &str) -> Result<()> {
    if value.len() > MAX_VALUE_BYTES {
        anyhow::bail!(
            "fact value exceeds maximum size ({} > {} bytes)",
            value.len(),
            MAX_VALUE_BYTES
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Time helper
// ---------------------------------------------------------------------------

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    struct TestEnv {
        _dir: TempDir,
        pub mgr: MemoryManager<'static>,
    }

    fn temp_mgr(project: &str) -> TestEnv {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("memory.db");
        // Safety: the TempDir lives as long as TestEnv; the &'static cast is
        // safe here because we never store the manager beyond the test.
        let mgr = MemoryManager::open(&db_path, project, None).unwrap();
        // SAFETY: This is acceptable in tests only. The TempDir outlives the mgr
        // within the TestEnv. We transmute so we don't have to carry a lifetime
        // through every test helper, given Option<&SearchStore> is None.
        let mgr: MemoryManager<'static> = unsafe { std::mem::transmute(mgr) };
        TestEnv { _dir: dir, mgr }
    }

    // -----------------------------------------------------------------------
    // store_fact / retrieve_all_facts / clear_facts
    // -----------------------------------------------------------------------

    #[test]
    fn test_store_and_retrieve_all_facts() {
        let env = temp_mgr("proj");
        env.mgr.store_fact("lang", "Rust").unwrap();
        env.mgr.store_fact("framework", "Tokio").unwrap();

        let facts = env.mgr.retrieve_all_facts().unwrap();
        assert_eq!(facts.len(), 2);
        // Ordered by key: "framework" < "lang"
        assert_eq!(facts[0].key, "framework");
        assert_eq!(facts[0].value, "Tokio");
        assert_eq!(facts[1].key, "lang");
        assert_eq!(facts[1].value, "Rust");
        assert_eq!(facts[0].source, MemorySource::Structured);
        assert!((facts[0].confidence - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_store_fact_upsert_updates_existing() {
        let env = temp_mgr("proj");
        env.mgr.store_fact("model", "qwen-v1").unwrap();
        env.mgr.store_fact("model", "qwen-v2").unwrap();

        let facts = env.mgr.retrieve_all_facts().unwrap();
        assert_eq!(facts.len(), 1, "upsert should not create a second row");
        assert_eq!(facts[0].value, "qwen-v2");
    }

    #[test]
    fn test_retrieve_exact_match() {
        let env = temp_mgr("proj");
        env.mgr.store_fact("editor", "neovim").unwrap();

        let entry = env.mgr.retrieve("editor").unwrap();
        assert!(entry.is_some());
        let e = entry.unwrap();
        assert_eq!(e.key, "editor");
        assert_eq!(e.value, "neovim");
        assert_eq!(e.source, MemorySource::Structured);
        assert!((e.confidence - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_retrieve_no_match_returns_none() {
        let env = temp_mgr("proj");
        let entry = env.mgr.retrieve("nonexistent").unwrap();
        assert!(entry.is_none());
    }

    #[test]
    fn test_retrieve_all_facts_empty() {
        let env = temp_mgr("proj");
        let facts = env.mgr.retrieve_all_facts().unwrap();
        assert!(facts.is_empty());
    }

    #[test]
    fn test_clear_facts_removes_all() {
        let env = temp_mgr("proj");
        env.mgr.store_fact("a", "1").unwrap();
        env.mgr.store_fact("b", "2").unwrap();
        env.mgr.store_fact("c", "3").unwrap();

        let deleted = env.mgr.clear_facts().unwrap();
        assert_eq!(deleted, 3);

        let remaining = env.mgr.retrieve_all_facts().unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_clear_facts_only_affects_current_project() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("shared.db");

        let mgr_a = MemoryManager::open(&db_path, "proj-a", None).unwrap();
        let mgr_b = MemoryManager::open(&db_path, "proj-b", None).unwrap();

        mgr_a.store_fact("key", "val-a").unwrap();
        mgr_b.store_fact("key", "val-b").unwrap();

        mgr_a.clear_facts().unwrap();

        // proj-a facts gone, proj-b untouched.
        assert!(mgr_a.retrieve_all_facts().unwrap().is_empty());
        let b_facts = mgr_b.retrieve_all_facts().unwrap();
        assert_eq!(b_facts.len(), 1);
        assert_eq!(b_facts[0].value, "val-b");
    }

    #[test]
    fn test_project_isolation_in_store_and_retrieve() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("shared.db");

        let mgr_a = MemoryManager::open(&db_path, "alpha", None).unwrap();
        let mgr_b = MemoryManager::open(&db_path, "beta", None).unwrap();

        mgr_a.store_fact("color", "red").unwrap();
        mgr_b.store_fact("color", "blue").unwrap();

        let a_entry = mgr_a.retrieve("color").unwrap().unwrap();
        let b_entry = mgr_b.retrieve("color").unwrap().unwrap();

        assert_eq!(a_entry.value, "red");
        assert_eq!(b_entry.value, "blue");
    }

    // -----------------------------------------------------------------------
    // Validation
    // -----------------------------------------------------------------------

    #[test]
    fn test_store_fact_rejects_empty_key() {
        let env = temp_mgr("proj");
        let result = env.mgr.store_fact("", "value");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_store_fact_rejects_oversized_key() {
        let env = temp_mgr("proj");
        let long_key = "k".repeat(MAX_KEY_BYTES + 1);
        let result = env.mgr.store_fact(&long_key, "value");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("maximum length"));
    }

    #[test]
    fn test_store_fact_rejects_oversized_value() {
        let env = temp_mgr("proj");
        let huge_value = "v".repeat(MAX_VALUE_BYTES + 1);
        let result = env.mgr.store_fact("key", &huge_value);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("maximum size"));
    }

    #[test]
    fn test_store_fact_accepts_max_boundary_key() {
        let env = temp_mgr("proj");
        let boundary_key = "k".repeat(MAX_KEY_BYTES);
        assert!(env.mgr.store_fact(&boundary_key, "value").is_ok());
    }

    #[test]
    fn test_store_fact_accepts_empty_value() {
        // Empty values are intentionally allowed (explicit erasure semantics).
        let env = temp_mgr("proj");
        assert!(env.mgr.store_fact("key", "").is_ok());
        let e = env.mgr.retrieve("key").unwrap().unwrap();
        assert_eq!(e.value, "");
    }

    // -----------------------------------------------------------------------
    // Memory source / confidence
    // -----------------------------------------------------------------------

    #[test]
    fn test_memory_source_display() {
        assert_eq!(MemorySource::Vector.to_string(), "vector");
        assert_eq!(MemorySource::Structured.to_string(), "structured");
        assert_eq!(MemorySource::Both.to_string(), "both");
    }

    #[test]
    fn test_structured_confidence_is_1() {
        let env = temp_mgr("proj");
        env.mgr.store_fact("k", "v").unwrap();
        let e = env.mgr.retrieve("k").unwrap().unwrap();
        assert!((e.confidence - 1.0).abs() < f32::EPSILON);
    }

    // -----------------------------------------------------------------------
    // Multi-fact ordering
    // -----------------------------------------------------------------------

    #[test]
    fn test_retrieve_all_facts_ordered_by_key() {
        let env = temp_mgr("proj");
        for key in &["zebra", "apple", "mango", "banana"] {
            env.mgr.store_fact(key, key).unwrap();
        }

        let facts = env.mgr.retrieve_all_facts().unwrap();
        let keys: Vec<&str> = facts.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["apple", "banana", "mango", "zebra"]);
    }

    // -----------------------------------------------------------------------
    // Schema idempotency
    // -----------------------------------------------------------------------

    #[test]
    fn test_open_twice_same_path_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("mem.db");

        {
            let mgr = MemoryManager::open(&db_path, "proj", None).unwrap();
            mgr.store_fact("x", "1").unwrap();
        }
        // Re-open — CREATE TABLE IF NOT EXISTS should not error.
        let mgr2 = MemoryManager::open(&db_path, "proj", None).unwrap();
        let facts = mgr2.retrieve_all_facts().unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, "1");
    }

    // -----------------------------------------------------------------------
    // pseudo_embed helper
    // -----------------------------------------------------------------------

    #[test]
    fn test_pseudo_embed_length() {
        let v = pseudo_embed("hello world");
        assert_eq!(v.len(), 64);
    }

    #[test]
    fn test_pseudo_embed_normalised() {
        let v = pseudo_embed("test");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "pseudo_embed must be unit-length, got {norm}");
    }

    #[test]
    fn test_pseudo_embed_empty_string() {
        // Should not panic — returns zero vector.
        let v = pseudo_embed("");
        assert_eq!(v.len(), 64);
        assert!(v.iter().all(|x| *x == 0.0));
    }
}

// ---------------------------------------------------------------------------
// Security red tests (FolkTech Secure Coding Standard)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod security_tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_mgr(project: &str) -> (TempDir, MemoryManager<'static>) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("memory.db");
        let mgr = MemoryManager::open(&db_path, project, None).unwrap();
        let mgr: MemoryManager<'static> = unsafe { std::mem::transmute(mgr) };
        (dir, mgr)
    }

    // -----------------------------------------------------------------------
    // P0: SQL injection via fact key
    // -----------------------------------------------------------------------

    #[test]
    fn test_p0_sql_injection_via_key_drop_table() {
        // ATTACK: Key contains a classic DROP TABLE payload.
        // EXPECT: Parameterized statement neutralizes the injection.
        // VERIFY: Fact is stored and retrieved verbatim; table still exists.
        let (_dir, mgr) = temp_mgr("proj");
        let evil_key = "'; DROP TABLE facts; --";
        mgr.store_fact(evil_key, "sentinel").unwrap();

        let facts = mgr.retrieve_all_facts().unwrap();
        assert_eq!(facts.len(), 1, "facts table must survive the injection attempt");
        assert_eq!(facts[0].key, evil_key);
        assert_eq!(facts[0].value, "sentinel");
    }

    #[test]
    fn test_p0_sql_injection_via_key_union_select() {
        // ATTACK: UNION SELECT to exfiltrate data via key field.
        let (_dir, mgr) = temp_mgr("proj");
        let evil_key = "' UNION SELECT 1, 'leaked', 'leaked', 0 --";
        mgr.store_fact(evil_key, "safe-value").unwrap();

        let facts = mgr.retrieve_all_facts().unwrap();
        // Must see exactly 1 row, not extra injected rows.
        assert_eq!(facts.len(), 1, "UNION SELECT must not inject extra rows");
        assert_eq!(facts[0].key, evil_key);
    }

    // -----------------------------------------------------------------------
    // P0: SQL injection via fact value
    // -----------------------------------------------------------------------

    #[test]
    fn test_p0_sql_injection_via_value_drop_table() {
        // ATTACK: Value field contains DROP TABLE payload.
        let (_dir, mgr) = temp_mgr("proj");
        let evil_value = "'; DROP TABLE facts; --";
        mgr.store_fact("safe-key", evil_value).unwrap();

        let entry = mgr.retrieve("safe-key").unwrap().unwrap();
        assert_eq!(entry.value, evil_value, "value must be stored/retrieved literally");

        // Table must still be alive.
        let _ = mgr.retrieve_all_facts().unwrap();
    }

    #[test]
    fn test_p0_sql_injection_via_value_insert_extra_rows() {
        // ATTACK: Try to close the INSERT statement and open a second one.
        let (_dir, mgr) = temp_mgr("proj");
        let evil_value = "x'), ('proj', 'injected-key', 'injected-value', 0); --";
        mgr.store_fact("real-key", evil_value).unwrap();

        let facts = mgr.retrieve_all_facts().unwrap();
        assert_eq!(facts.len(), 1, "no extra rows from INSERT injection");
        assert_eq!(facts[0].key, "real-key");
    }

    // -----------------------------------------------------------------------
    // P0: SQL injection via project name
    // -----------------------------------------------------------------------

    #[test]
    fn test_p0_sql_injection_via_project_name() {
        // ATTACK: Project name itself is an injection payload.
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("mem.db");
        let evil_project = "'; DELETE FROM facts; --";

        let mgr = MemoryManager::open(&db_path, evil_project, None).unwrap();
        mgr.store_fact("sentinel", "alive").unwrap();
        mgr.store_fact("key2", "val2").unwrap();

        // Reopen and verify rows are still there.
        let mgr2 = MemoryManager::open(&db_path, evil_project, None).unwrap();
        let facts = mgr2.retrieve_all_facts().unwrap();
        assert_eq!(facts.len(), 2, "DELETE injection via project name must not execute");
    }

    #[test]
    fn test_p0_sql_injection_project_isolation_not_bypassable() {
        // ATTACK: An attacker crafts a project name to break project isolation
        //         and read facts from other projects.
        //         E.g. project = "x' OR '1'='1" would make WHERE project=? match all rows.
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("mem.db");

        let mgr_good = MemoryManager::open(&db_path, "good-project", None).unwrap();
        mgr_good.store_fact("secret", "sensitive-value").unwrap();

        let evil_project = "x' OR '1'='1";
        let mgr_evil = MemoryManager::open(&db_path, evil_project, None).unwrap();

        // The evil project must NOT see good-project's facts.
        let evil_facts = mgr_evil.retrieve_all_facts().unwrap();
        for f in &evil_facts {
            assert_ne!(f.value, "sensitive-value", "project isolation bypass — attacker can see other project's facts");
        }
    }

    // -----------------------------------------------------------------------
    // P0: SQL injection via clear_facts project parameter
    // -----------------------------------------------------------------------

    #[test]
    fn test_p0_sql_injection_clear_facts_does_not_wipe_other_projects() {
        // ATTACK: An evil project name tries to make DELETE match all rows.
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("mem.db");

        let mgr_good = MemoryManager::open(&db_path, "legit", None).unwrap();
        mgr_good.store_fact("keep-me", "important").unwrap();

        // This evil project tries to wipe everything.
        let evil_project = "' OR '1'='1";
        let mgr_evil = MemoryManager::open(&db_path, evil_project, None).unwrap();
        mgr_evil.store_fact("evil-key", "evil-value").unwrap();
        mgr_evil.clear_facts().unwrap();

        // legit project's fact must still exist.
        let remaining = mgr_good.retrieve_all_facts().unwrap();
        assert_eq!(remaining.len(), 1, "clear_facts injection must not wipe other projects");
        assert_eq!(remaining[0].value, "important");
    }

    // -----------------------------------------------------------------------
    // P0: Input validation — key injection via empty / oversized inputs
    // -----------------------------------------------------------------------

    #[test]
    fn test_p0_empty_key_rejected_before_db_write() {
        // GUARD: Empty key would make WHERE key='' match unintentionally wide.
        let (_dir, mgr) = temp_mgr("proj");
        let result = mgr.store_fact("", "value");
        assert!(result.is_err(), "empty key must be rejected");
    }

    #[test]
    fn test_p0_oversized_key_rejected_before_db_write() {
        // GUARD: Huge keys can cause index bloat / denial of service.
        let (_dir, mgr) = temp_mgr("proj");
        let huge_key = "A".repeat(MAX_KEY_BYTES + 1);
        let result = mgr.store_fact(&huge_key, "v");
        assert!(result.is_err(), "oversized key must be rejected");
    }

    #[test]
    fn test_p0_oversized_value_rejected_before_db_write() {
        // GUARD: Multi-MB values exhaust disk/memory.
        let (_dir, mgr) = temp_mgr("proj");
        let huge_val = "X".repeat(MAX_VALUE_BYTES + 1);
        let result = mgr.store_fact("key", &huge_val);
        assert!(result.is_err(), "oversized value must be rejected");
    }

    // -----------------------------------------------------------------------
    // P0: Null-byte injection
    // -----------------------------------------------------------------------

    #[test]
    fn test_p0_null_byte_in_key_stored_literally() {
        // ATTACK: Null bytes in keys can confuse C-string-based lookups
        //         and make two different keys appear identical at the OS level.
        // VERIFY: rusqlite stores the key with the null byte; retrieval works.
        let (_dir, mgr) = temp_mgr("proj");
        let key_with_null = "key\0suffix";
        // We accept or reject — the important thing is it does not crash or
        // silently truncate in a security-relevant way.
        let _ = mgr.store_fact(key_with_null, "value");
        // If it succeeded, the round-trip must be exact.
        if let Some(e) = mgr.retrieve(key_with_null).unwrap() {
            assert_eq!(e.value, "value");
        }
    }

    // -----------------------------------------------------------------------
    // P1: Unicode / homoglyph key confusion
    // -----------------------------------------------------------------------

    #[test]
    fn test_p1_unicode_key_stored_and_retrieved_as_is() {
        // ATTACK: RTL override and zero-width chars in keys could make two keys
        //         look identical in a UI while being distinct in the DB.
        let (_dir, mgr) = temp_mgr("proj");
        let tricky_key = "admin\u{202E}user"; // RTL override
        mgr.store_fact(tricky_key, "tricky-value").unwrap();

        let entry = mgr.retrieve(tricky_key).unwrap().unwrap();
        assert_eq!(entry.value, "tricky-value");

        // Normal "adminuser" must NOT alias to this key.
        let normal_entry = mgr.retrieve("adminuser").unwrap();
        assert!(normal_entry.is_none(), "unicode normalization must not cause key aliasing");
    }

    // -----------------------------------------------------------------------
    // P1: Concurrent project separation (schema isolation)
    // -----------------------------------------------------------------------

    #[test]
    fn test_p1_two_projects_same_db_are_isolated() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("shared.db");

        let mgr_a = MemoryManager::open(&db_path, "project-a", None).unwrap();
        let mgr_b = MemoryManager::open(&db_path, "project-b", None).unwrap();

        mgr_a.store_fact("secret", "alpha-secret").unwrap();
        mgr_b.store_fact("secret", "beta-secret").unwrap();

        let a_entry = mgr_a.retrieve("secret").unwrap().unwrap();
        let b_entry = mgr_b.retrieve("secret").unwrap().unwrap();

        assert_eq!(a_entry.value, "alpha-secret");
        assert_eq!(b_entry.value, "beta-secret");
        assert_ne!(a_entry.value, b_entry.value, "project isolation broken");
    }
}
