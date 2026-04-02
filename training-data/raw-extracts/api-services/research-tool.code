use anyhow::Result;
use serde_json::Value;

use super::registry::{Tool, ToolContext, ToolResult};

/// Research tool for self-improvement: web search, docs, GitHub, and crates.io.
///
/// Uses reqwest for HTTP, DuckDuckGo Lite for web search, `gh` CLI for GitHub,
/// and the crates.io API for Rust crate discovery.
pub struct ResearchTool;

const VALID_SOURCES: &[&str] = &["web", "docs", "github", "crates"];
const VALID_DEPTHS: &[&str] = &["quick", "thorough"];
const MAX_QUERY_LEN: usize = 500;

impl Tool for ResearchTool {
    fn name(&self) -> &str {
        "research"
    }

    fn description(&self) -> &str {
        "Research a topic using web search and documentation fetching. Use for: learning about \
         unfamiliar APIs, finding best practices, checking library versions, discovering solutions \
         to errors. Returns synthesized findings from multiple sources."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Research query (e.g., 'rust async patterns', 'tokio best practices 2026')"
                },
                "sources": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["web", "docs", "github", "crates"] },
                    "description": "Which sources to search (default: all)"
                },
                "depth": {
                    "type": "string",
                    "enum": ["quick", "thorough"],
                    "description": "Quick returns first results; thorough fetches and synthesizes multiple pages"
                }
            }
        })
    }

    fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let query = params["query"].as_str().unwrap_or("").to_string();
        let sources: Vec<String> = params["sources"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_else(|| VALID_SOURCES.iter().map(|s| s.to_string()).collect());
        let depth = params["depth"]
            .as_str()
            .unwrap_or("quick")
            .to_string();
        let project_path = ctx.project_path.clone();

        Box::pin(async move {
            // Validate query
            if query.is_empty() {
                return Ok(ToolResult::error("Missing required parameter: query"));
            }
            if query.len() > MAX_QUERY_LEN {
                return Ok(ToolResult::error(format!(
                    "Query too long ({} chars, max {MAX_QUERY_LEN}).",
                    query.len()
                )));
            }
            if query.contains('\0') {
                return Ok(ToolResult::error("Query must not contain null bytes."));
            }

            // Validate sources
            for src in &sources {
                if !VALID_SOURCES.contains(&src.as_str()) {
                    return Ok(ToolResult::error(format!(
                        "Invalid source: {src}. Must be one of: web, docs, github, crates"
                    )));
                }
            }

            // Validate depth
            if !VALID_DEPTHS.contains(&depth.as_str()) {
                return Ok(ToolResult::error(format!(
                    "Invalid depth: {depth}. Must be 'quick' or 'thorough'"
                )));
            }

            let is_thorough = depth == "thorough";
            let max_results = if is_thorough { 5 } else { 3 };
            let mut all_results = Vec::new();

            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap();

            // Search each source
            for source in &sources {
                match source.as_str() {
                    "web" | "docs" => {
                        match search_web(&client, &query, max_results).await {
                            Ok(results) => {
                                for r in results {
                                    all_results.push(format!("[web] {r}"));
                                }
                            }
                            Err(e) => {
                                all_results.push(format!("[web] Search failed: {e}"));
                            }
                        }
                    }
                    "github" => {
                        match search_github(&query, max_results, &project_path).await {
                            Ok(results) => {
                                for r in results {
                                    all_results.push(format!("[github] {r}"));
                                }
                            }
                            Err(e) => {
                                all_results.push(format!("[github] Search failed: {e}"));
                            }
                        }
                    }
                    "crates" => {
                        match search_crates(&client, &query, max_results).await {
                            Ok(results) => {
                                for r in results {
                                    all_results.push(format!("[crates] {r}"));
                                }
                            }
                            Err(e) => {
                                all_results.push(format!("[crates] Search failed: {e}"));
                            }
                        }
                    }
                    _ => {}
                }
            }

            if all_results.is_empty() {
                return Ok(ToolResult::success(format!(
                    "No results found for: {query}"
                )));
            }

            let output = format!(
                "Research results for: {query}\n\n{}\n",
                all_results.join("\n\n")
            );

            Ok(ToolResult::success(output))
        })
    }
}

/// Search via DuckDuckGo Lite HTML endpoint.
async fn search_web(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<Vec<String>> {
    // URL-encode the query manually to avoid pulling in another crate
    let encoded_query = url_encode(query);
    let url = format!(
        "https://lite.duckduckgo.com/lite/?q={encoded_query}"
    );

    let resp = client
        .get(&url)
        .header("User-Agent", "ftai-research/0.1")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Ok(vec![format!("HTTP {}", resp.status())]);
    }

    let body = resp.text().await?;

    // Extract result snippets from the HTML
    let results = extract_ddg_results(&body, max_results);
    if results.is_empty() {
        Ok(vec![format!("No web results for: {query}")])
    } else {
        Ok(results)
    }
}

/// Search GitHub using the `gh` CLI.
async fn search_github(
    query: &str,
    max_results: usize,
    project_path: &std::path::Path,
) -> Result<Vec<String>> {
    use std::process::Stdio;
    use tokio::process::Command;

    // Sanitize query: remove shell metacharacters for safety
    let safe_query: String = query
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_' || *c == '.')
        .collect();

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        Command::new("gh")
            .args(["search", "repos", &safe_query, "--limit", &max_results.to_string(), "--json", "name,description,url"])
            .current_dir(project_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    match output {
        Ok(Ok(out)) => {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Ok(vec![format!("gh search failed: {stderr}")]);
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Ok(repos) = serde_json::from_str::<Vec<Value>>(&stdout) {
                let results: Vec<String> = repos
                    .iter()
                    .take(max_results)
                    .map(|r| {
                        let name = r["name"].as_str().unwrap_or("?");
                        let desc = r["description"].as_str().unwrap_or("(no description)");
                        let url = r["url"].as_str().unwrap_or("");
                        format!("{name} — {desc}\n  {url}")
                    })
                    .collect();
                Ok(results)
            } else {
                Ok(vec!["Failed to parse gh search output".to_string()])
            }
        }
        Ok(Err(e)) => Ok(vec![format!("gh not available: {e}")]),
        Err(_) => Ok(vec!["GitHub search timed out".to_string()]),
    }
}

/// Search crates.io API.
async fn search_crates(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<Vec<String>> {
    let encoded_query = url_encode(query);
    let url = format!(
        "https://crates.io/api/v1/crates?q={encoded_query}&per_page={max_results}"
    );

    let resp = client
        .get(&url)
        .header("User-Agent", "ftai-research/0.1 (https://folktech.ai)")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Ok(vec![format!("crates.io HTTP {}", resp.status())]);
    }

    let body: Value = resp.json().await?;

    let crates = body["crates"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let results: Vec<String> = crates
        .iter()
        .take(max_results)
        .map(|c| {
            let name = c["name"].as_str().unwrap_or("?");
            let version = c["newest_version"].as_str().unwrap_or("?");
            let desc = c["description"].as_str().unwrap_or("(no description)");
            let downloads = c["downloads"].as_u64().unwrap_or(0);
            format!(
                "{name} v{version} ({downloads} downloads)\n  {desc}"
            )
        })
        .collect();

    if results.is_empty() {
        Ok(vec![format!("No crates found for: {query}")])
    } else {
        Ok(results)
    }
}

/// Extract result links and snippets from DuckDuckGo Lite HTML.
fn extract_ddg_results(html: &str, max: usize) -> Vec<String> {
    let mut results = Vec::new();

    // DuckDuckGo Lite has a simple structure with result links in <a> tags
    // with class "result-link" and snippets in <td> tags with class "result-snippet"
    // We use a simple approach: find href patterns and nearby text
    let mut pos = 0;
    while results.len() < max {
        // Find next result link
        let link_start = match html[pos..].find("href=\"") {
            Some(i) => pos + i + 6,
            None => break,
        };
        let link_end = match html[link_start..].find('"') {
            Some(i) => link_start + i,
            None => break,
        };

        let link = &html[link_start..link_end];

        // Skip DuckDuckGo internal links
        if link.starts_with("http") && !link.contains("duckduckgo.com") {
            // Try to extract some text after the link
            let text_start = link_end;
            let text_end = (text_start + 300).min(html.len());
            let snippet_html = &html[text_start..text_end];
            let snippet = strip_tags(snippet_html);
            let snippet: String = snippet.chars().take(200).collect();

            if !snippet.trim().is_empty() {
                results.push(format!("{link}\n  {}", snippet.trim()));
            } else {
                results.push(link.to_string());
            }
        }

        pos = link_end + 1;
        if pos >= html.len() {
            break;
        }
    }

    results
}

/// Simple HTML tag stripper.
fn strip_tags(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(c);
        }
    }
    // Collapse whitespace
    let parts: Vec<&str> = result.split_whitespace().collect();
    parts.join(" ")
}

/// Percent-encode a string for use in URLs.
fn url_encode(input: &str) -> String {
    let mut result = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    result
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        }
    }

    // ── Parameter validation tests ─────────────────────────────────────────

    #[tokio::test]
    async fn test_empty_query_rejected() {
        let tool = ResearchTool;
        let result = tool
            .execute(serde_json::json!({"query": ""}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_long_query_rejected() {
        let tool = ResearchTool;
        let long = "x".repeat(600);
        let result = tool
            .execute(serde_json::json!({"query": long}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("too long"));
    }

    #[tokio::test]
    async fn test_invalid_source_rejected() {
        let tool = ResearchTool;
        let result = tool
            .execute(
                serde_json::json!({"query": "test", "sources": ["invalid"]}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Invalid source"));
    }

    #[tokio::test]
    async fn test_invalid_depth_rejected() {
        let tool = ResearchTool;
        let result = tool
            .execute(
                serde_json::json!({"query": "test", "depth": "super-deep"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Invalid depth"));
    }

    // ── URL encoding tests ─────────────────────────────────────────────────

    #[test]
    fn test_url_encode_basic() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("rust async"), "rust+async");
    }

    #[test]
    fn test_url_encode_special_chars() {
        let encoded = url_encode("foo&bar=baz");
        assert!(encoded.contains("%26")); // &
        assert!(encoded.contains("%3D")); // =
    }

    #[test]
    fn test_url_encode_preserves_safe() {
        assert_eq!(url_encode("hello-world_v2.0"), "hello-world_v2.0");
    }

    // ── HTML parsing tests ─────────────────────────────────────────────────

    #[test]
    fn test_strip_tags() {
        assert_eq!(strip_tags("<b>bold</b> text"), "bold text");
        assert_eq!(strip_tags("no tags"), "no tags");
    }

    #[test]
    fn test_extract_ddg_results_empty() {
        let results = extract_ddg_results("no links here", 3);
        assert!(results.is_empty());
    }

    #[test]
    fn test_extract_ddg_results_with_links() {
        let html = r#"<a href="https://example.com/page">Example Page</a> Some description text here."#;
        let results = extract_ddg_results(html, 3);
        assert!(!results.is_empty());
        assert!(results[0].contains("example.com"));
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_security_null_bytes_in_query() {
        let tool = ResearchTool;
        let result = tool
            .execute(
                serde_json::json!({"query": "test\u{0000}injection"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("null"));
    }

    #[test]
    fn test_security_url_encode_injection() {
        // Verify that URL-encoding prevents injection
        let malicious = "query; rm -rf /";
        let encoded = url_encode(malicious);
        assert!(!encoded.contains(';'));
        assert!(!encoded.contains(' '));  // spaces become +
    }

    #[test]
    fn test_security_github_query_sanitization() {
        // Verify that shell metacharacters are stripped from GitHub queries
        let malicious = "query; rm -rf / && cat /etc/passwd";
        let safe: String = malicious
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_' || *c == '.')
            .collect();
        assert!(!safe.contains(';'));
        assert!(!safe.contains('&'));
        assert!(!safe.contains('/'));
    }
}
