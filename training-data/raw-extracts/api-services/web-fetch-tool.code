use anyhow::Result;
use serde_json::Value;

use super::registry::{Tool, ToolContext, ToolResult};

pub struct WebFetchTool;

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL and return it as text/markdown."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                }
            }
        })
    }

    fn execute(
        &self,
        params: Value,
        _ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let url = params["url"].as_str().unwrap_or("").to_string();

        Box::pin(async move {
            if url.is_empty() {
                return Ok(ToolResult::error("No URL provided"));
            }

            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap();

            match client.get(&url).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        return Ok(ToolResult::error(format!("HTTP {status}")));
                    }

                    let content_type = resp
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();

                    let body = match resp.text().await {
                        Ok(b) => b,
                        Err(e) => return Ok(ToolResult::error(format!("Failed to read body: {e}"))),
                    };

                    // Basic HTML stripping for HTML responses
                    let output = if content_type.contains("html") {
                        strip_html_basic(&body)
                    } else {
                        body
                    };

                    // Truncate very large responses
                    if output.len() > 50_000 {
                        Ok(ToolResult::success(format!(
                            "{}\n\n... (truncated, {} total chars)",
                            &output[..50_000],
                            output.len()
                        )))
                    } else {
                        Ok(ToolResult::success(output))
                    }
                }
                Err(e) => Ok(ToolResult::error(format!("Request failed: {e}"))),
            }
        })
    }
}

/// Basic HTML tag stripping (not a full parser — good enough for reading docs)
fn strip_html_basic(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut in_script = false;

    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            // Check for script/style tags
            let rest: String = html[html.len().saturating_sub(html.len())..].chars().take(10).collect();
            if rest.starts_with("<script") || rest.starts_with("<style") {
                in_script = true;
            }
            continue;
        }
        if c == '>' {
            in_tag = false;
            if in_script {
                in_script = false;
            }
            continue;
        }
        if !in_tag && !in_script {
            result.push(c);
        }
    }

    // Clean up whitespace
    let lines: Vec<&str> = result
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html() {
        let html = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let result = strip_html_basic(html);
        assert!(result.contains("Hello"));
        assert!(result.contains("World"));
        assert!(!result.contains("<"));
    }

    #[test]
    fn test_strip_html_preserves_text() {
        let html = "Just plain text";
        assert_eq!(strip_html_basic(html), "Just plain text");
    }
}
