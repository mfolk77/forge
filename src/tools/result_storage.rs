use std::path::Path;

const MAX_RESULT_CHARS: usize = 50_000;
const PREVIEW_CHARS: usize = 500;

/// If a tool result exceeds MAX_RESULT_CHARS, persist the full output to disk
/// and return a preview with a file reference. Small results pass through unchanged.
///
/// The `tool_name` is sanitized to prevent path traversal.
pub fn maybe_persist_result(result: &str, tool_name: &str, project_path: &Path) -> String {
    if result.len() <= MAX_RESULT_CHARS {
        return result.to_string();
    }

    let dir = project_path.join(".ftai").join("tool-results");
    if std::fs::create_dir_all(&dir).is_err() {
        // Can't persist — fall back to truncated preview
        return truncated_preview(result);
    }

    // Sanitize tool_name: strip path separators and traversal
    let safe_name: String = tool_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();

    let id = uuid::Uuid::new_v4();
    let short_id: String = id.to_string().chars().take(8).collect();
    let filename = format!("{safe_name}_{short_id}.txt");
    let path = dir.join(&filename);

    // Verify path is inside tool-results dir (defense in depth)
    if let (Ok(canonical_dir), Ok(canonical_path)) = (
        std::fs::canonicalize(&dir),
        {
            // Write first so canonicalize works
            let _ = std::fs::write(&path, result);
            std::fs::canonicalize(&path)
        },
    ) {
        if !canonical_path.starts_with(&canonical_dir) {
            // Path traversal detected — remove and return preview
            let _ = std::fs::remove_file(&path);
            return truncated_preview(result);
        }
    }

    let preview_end = PREVIEW_CHARS.min(result.len());
    // Ensure we don't split in the middle of a UTF-8 character
    let preview = &result[..FloorCharBoundary::floor_char_boundary(result, preview_end)];

    format!(
        "<persisted-output>\nOutput too large ({} chars). Full output saved to: {}\n\nPreview (first {} chars):\n{}\n</persisted-output>",
        result.len(),
        path.display(),
        preview.len(),
        preview
    )
}

fn truncated_preview(result: &str) -> String {
    let end = PREVIEW_CHARS.min(result.len());
    let preview = &result[..FloorCharBoundary::floor_char_boundary(result, end)];
    format!(
        "[Output too large ({} chars), could not persist to disk]\nPreview:\n{}...",
        result.len(),
        preview
    )
}

/// Helper trait for floor_char_boundary on stable Rust < 1.73.
/// On newer Rust this is available as str::floor_char_boundary.
trait FloorCharBoundary {
    fn floor_char_boundary(&self, index: usize) -> usize;
}

impl FloorCharBoundary for str {
    fn floor_char_boundary(&self, index: usize) -> usize {
        if index >= self.len() {
            return self.len();
        }
        let mut i = index;
        while i > 0 && !self.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_small_result_passes_through() {
        let tmp = TempDir::new().unwrap();
        let result = "hello world";
        let output = maybe_persist_result(result, "bash", tmp.path());
        assert_eq!(output, "hello world");
    }

    #[test]
    fn test_large_result_persisted_to_disk() {
        let tmp = TempDir::new().unwrap();
        let large = "x".repeat(60_000);
        let output = maybe_persist_result(&large, "bash", tmp.path());
        assert!(output.contains("persisted-output"));
        assert!(output.contains("60000 chars"));
        assert!(output.contains(".ftai/tool-results/"));

        // Verify file exists on disk
        let results_dir = tmp.path().join(".ftai").join("tool-results");
        let entries: Vec<_> = std::fs::read_dir(&results_dir)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(entries.len(), 1);

        // Verify full content was written
        let written = std::fs::read_to_string(entries[0].path()).unwrap();
        assert_eq!(written.len(), 60_000);
    }

    #[test]
    fn test_preview_included_in_persisted_output() {
        let tmp = TempDir::new().unwrap();
        let large = format!("HEADER_CONTENT{}", "x".repeat(60_000));
        let output = maybe_persist_result(&large, "grep", tmp.path());
        assert!(output.contains("HEADER_CONTENT"));
    }

    #[test]
    fn test_exactly_at_threshold_passes_through() {
        let tmp = TempDir::new().unwrap();
        let result = "x".repeat(MAX_RESULT_CHARS);
        let output = maybe_persist_result(&result, "bash", tmp.path());
        assert_eq!(output, result);
    }

    #[test]
    fn test_one_over_threshold_persists() {
        let tmp = TempDir::new().unwrap();
        let result = "x".repeat(MAX_RESULT_CHARS + 1);
        let output = maybe_persist_result(&result, "bash", tmp.path());
        assert!(output.contains("persisted-output"));
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[test]
    fn test_p0_tool_name_path_traversal() {
        let tmp = TempDir::new().unwrap();
        let large = "x".repeat(60_000);

        // Attempt path traversal via tool name
        let output = maybe_persist_result(&large, "../../etc/passwd", tmp.path());
        // Should still succeed but with sanitized filename
        assert!(output.contains("persisted-output"));

        // Verify the file was written inside tool-results, not outside
        let results_dir = tmp.path().join(".ftai").join("tool-results");
        let entries: Vec<_> = std::fs::read_dir(&results_dir)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(entries.len(), 1);
        let name = entries[0].file_name().to_string_lossy().to_string();
        assert!(!name.contains(".."));
        assert!(!name.contains('/'));

        // Verify nothing was written outside the tool-results dir
        assert!(!tmp.path().join("etc").exists());
    }

    #[test]
    fn test_p0_tool_name_with_slashes_sanitized() {
        let tmp = TempDir::new().unwrap();
        let large = "x".repeat(60_000);
        let output = maybe_persist_result(&large, "tool/../../secret", tmp.path());
        assert!(output.contains("persisted-output"));

        let results_dir = tmp.path().join(".ftai").join("tool-results");
        let entries: Vec<_> = std::fs::read_dir(&results_dir)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(entries.len(), 1);
        let name = entries[0].file_name().to_string_lossy().to_string();
        assert!(!name.contains('/'));
        assert!(!name.contains(".."));
    }
}
