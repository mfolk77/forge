use anyhow::{Context, Result};

use super::indexer::{ChunkType, CodeIndexer};
use super::store::SearchStore;

/// A single search result with relevance score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file_path: String,
    pub symbol_name: Option<String>,
    pub chunk_type: ChunkType,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    /// Cosine similarity score in `[0, 1]` (higher = more relevant).
    pub score: f32,
}

/// Semantic code search engine. Embeds the query, then performs brute-force
/// cosine similarity against all stored chunk embeddings.
#[derive(Debug)]
pub struct SearchEngine<'a> {
    indexer: &'a CodeIndexer,
}

impl<'a> SearchEngine<'a> {
    pub fn new(indexer: &'a CodeIndexer) -> Self {
        Self { indexer }
    }

    /// Search for chunks semantically similar to `query`.
    ///
    /// `top_k` limits how many results to return.
    /// `file_filter` optionally restricts results to paths containing the given substring.
    pub fn search(
        &self,
        query: &str,
        top_k: usize,
        file_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let store = self.indexer.store();

        // Embed the query text using the same model.
        let query_embeddings = self
            .indexer
            .embed_query(query)
            .context("failed to embed search query")?;
        let query_emb = &query_embeddings;

        // Load all embeddings from the store.
        let (ids, embeddings) = store.all_embeddings()?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        // Score every chunk.
        let mut scored: Vec<(i64, f32)> = ids
            .iter()
            .zip(embeddings.iter())
            .map(|(&id, emb)| (id, cosine_similarity(query_emb, emb)))
            .collect();

        // Sort descending by score.
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Collect top-k results, applying optional file filter.
        let mut results = Vec::with_capacity(top_k);
        for (id, score) in scored {
            if results.len() >= top_k {
                break;
            }
            let chunk = store.get_chunk(id)?;

            if let Some(filter) = file_filter {
                if !chunk.file_path.contains(filter) {
                    continue;
                }
            }

            results.push(SearchResult {
                file_path: chunk.file_path,
                symbol_name: chunk.symbol_name,
                chunk_type: ChunkType::from_str(&chunk.chunk_type),
                start_line: chunk.start_line,
                end_line: chunk.end_line,
                content: chunk.content,
                score,
            });
        }

        Ok(results)
    }
}

/// Standalone search function that works directly with a store, query embedding,
/// and optional file filter. Useful when you already have the embedding.
pub fn search_with_embedding(
    store: &SearchStore,
    query_emb: &[f32],
    top_k: usize,
    file_filter: Option<&str>,
) -> Result<Vec<SearchResult>> {
    let (ids, embeddings) = store.all_embeddings()?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut scored: Vec<(i64, f32)> = ids
        .iter()
        .zip(embeddings.iter())
        .map(|(&id, emb)| (id, cosine_similarity(query_emb, emb)))
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut results = Vec::with_capacity(top_k);
    for (id, score) in scored {
        if results.len() >= top_k {
            break;
        }
        let chunk = store.get_chunk(id)?;

        if let Some(filter) = file_filter {
            if !chunk.file_path.contains(filter) {
                continue;
            }
        }

        results.push(SearchResult {
            file_path: chunk.file_path,
            symbol_name: chunk.symbol_name,
            chunk_type: ChunkType::from_str(&chunk.chunk_type),
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            content: chunk.content,
            score,
        });
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Cosine similarity
// ---------------------------------------------------------------------------

/// Compute cosine similarity between two f32 slices.
///
/// Returns 0.0 if either vector has zero magnitude.
/// Written as a tight loop over slices for auto-vectorization.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 0.0;
    }

    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;

    // Hot loop — compiler will auto-vectorize this with SIMD.
    for i in 0..len {
        let ai = a[i];
        let bi = b[i];
        dot += ai * bi;
        norm_a += ai * ai;
        norm_b += bi * bi;
    }

    let denom = (norm_a * norm_b).sqrt();
    if denom < f32::EPSILON {
        0.0
    } else {
        dot / denom
    }
}

// ---------------------------------------------------------------------------
// Result formatting
// ---------------------------------------------------------------------------

/// Format search results as a string suitable for injection into an LLM context.
pub fn format_results(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return String::from("No search results found.");
    }

    let mut buf = String::with_capacity(results.len() * 256);
    for (i, r) in results.iter().enumerate() {
        buf.push_str(&format!(
            "--- Result {} (score: {:.3}) ---\n",
            i + 1,
            r.score
        ));
        buf.push_str(&format!(
            "File: {} (lines {}-{})\n",
            r.file_path, r.start_line, r.end_line
        ));
        if let Some(ref name) = r.symbol_name {
            buf.push_str(&format!("Symbol: {} ({})\n", name, r.chunk_type));
        }
        buf.push_str(&r.content);
        buf.push_str("\n\n");
    }
    buf
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_identical_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let score = cosine_similarity(&a, &a);
        assert!((score - 1.0).abs() < 1e-5, "identical vectors should have similarity ~1.0, got {score}");
    }

    #[test]
    fn test_cosine_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let score = cosine_similarity(&a, &b);
        assert!(score.abs() < 1e-5, "orthogonal vectors should have similarity ~0.0, got {score}");
    }

    #[test]
    fn test_cosine_opposite_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let score = cosine_similarity(&a, &b);
        assert!((score + 1.0).abs() < 1e-5, "opposite vectors should have similarity ~-1.0, got {score}");
    }

    #[test]
    fn test_cosine_zero_vector() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![0.0, 0.0, 0.0];
        let score = cosine_similarity(&a, &b);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_cosine_empty_vectors() {
        let score = cosine_similarity(&[], &[]);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_cosine_mismatched_lengths() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b = vec![1.0, 2.0];
        // Should use the shorter length.
        let score = cosine_similarity(&a, &b);
        // Manual: dot=5, norm_a=5, norm_b=5, sim = 5/5 = 1.0
        assert!((score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_known_value() {
        let a = vec![1.0, 0.0];
        let b = vec![1.0, 1.0];
        // cos = 1 / (1 * sqrt(2)) = 0.7071...
        let score = cosine_similarity(&a, &b);
        assert!((score - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_unit_vectors() {
        // Pre-normalised unit vectors.
        let a = vec![0.6, 0.8];
        let b = vec![0.8, 0.6];
        let score = cosine_similarity(&a, &b);
        // dot = 0.48 + 0.48 = 0.96
        assert!((score - 0.96).abs() < 1e-4);
    }

    #[test]
    fn test_format_results_empty() {
        assert_eq!(format_results(&[]), "No search results found.");
    }

    #[test]
    fn test_format_results_single() {
        let results = vec![SearchResult {
            file_path: "src/main.rs".to_string(),
            symbol_name: Some("main".to_string()),
            chunk_type: ChunkType::Function,
            start_line: 1,
            end_line: 5,
            content: "fn main() {}".to_string(),
            score: 0.95,
        }];
        let formatted = format_results(&results);
        assert!(formatted.contains("Result 1"));
        assert!(formatted.contains("0.950"));
        assert!(formatted.contains("src/main.rs"));
        assert!(formatted.contains("Symbol: main"));
        assert!(formatted.contains("fn main()"));
    }

    #[test]
    fn test_format_results_no_symbol() {
        let results = vec![SearchResult {
            file_path: "data.txt".to_string(),
            symbol_name: None,
            chunk_type: ChunkType::Block,
            start_line: 1,
            end_line: 40,
            content: "some text".to_string(),
            score: 0.5,
        }];
        let formatted = format_results(&results);
        assert!(!formatted.contains("Symbol:"));
    }

    #[test]
    fn test_search_with_embedding_empty_store() {
        // Use a temp store with no chunks.
        let dir = tempfile::TempDir::new().unwrap();
        let store = SearchStore::open(&dir.path().join("empty.db")).unwrap();
        let results = search_with_embedding(&store, &[1.0, 0.0], 5, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_with_embedding_ranks_correctly() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = SearchStore::open(&dir.path().join("rank.db")).unwrap();

        // Insert two chunks with known embeddings.
        let close_emb = vec![0.9, 0.1];
        let far_emb = vec![0.1, 0.9];
        store
            .upsert_chunks(
                "close.rs",
                &[("Function", Some("close_fn"), 1, 5, "close code", &close_emb)],
                1.0,
            )
            .unwrap();
        store
            .upsert_chunks(
                "far.rs",
                &[("Function", Some("far_fn"), 1, 5, "far code", &far_emb)],
                1.0,
            )
            .unwrap();

        let query_emb = vec![1.0, 0.0]; // Closer to close_emb.
        let results = search_with_embedding(&store, &query_emb, 10, None).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].file_path, "close.rs");
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_search_with_file_filter() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = SearchStore::open(&dir.path().join("filter.db")).unwrap();

        let emb = vec![1.0, 0.0];
        store
            .upsert_chunks("src/foo.rs", &[("Function", Some("f"), 1, 5, "code", &emb)], 1.0)
            .unwrap();
        store
            .upsert_chunks("tests/bar.rs", &[("Function", Some("g"), 1, 5, "code", &emb)], 1.0)
            .unwrap();

        let results = search_with_embedding(&store, &emb, 10, Some("tests/")).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file_path, "tests/bar.rs");
    }

}
