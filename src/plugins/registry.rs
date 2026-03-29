use anyhow::{Context, Result};
use serde::Deserialize;

const DEFAULT_REGISTRY_URL: &str = "https://raw.githubusercontent.com/FolkTechAI/ftai-registry/main/registry.json";

#[derive(Debug, Clone, Deserialize)]
pub struct RegistryIndex {
    pub plugins: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegistryEntry {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

pub struct RegistryClient {
    url: String,
}

impl RegistryClient {
    pub fn new(url: Option<&str>) -> Self {
        Self {
            url: url.unwrap_or(DEFAULT_REGISTRY_URL).to_string(),
        }
    }

    /// Fetch the registry index from the remote URL.
    pub async fn fetch_index(&self) -> Result<RegistryIndex> {
        let resp = reqwest::get(&self.url)
            .await
            .context("Failed to fetch plugin registry")?;

        let body = resp
            .text()
            .await
            .context("Failed to read registry response")?;

        let index: RegistryIndex = serde_json::from_str(&body)
            .context("Failed to parse plugin registry JSON")?;

        Ok(index)
    }

    /// Search the registry for plugins matching a query.
    pub async fn search(&self, query: &str) -> Result<Vec<RegistryEntry>> {
        let index = self.fetch_index().await?;
        let query_lower = query.to_lowercase();

        let results: Vec<RegistryEntry> = index
            .plugins
            .into_iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&query_lower)
                    || p.description.to_lowercase().contains(&query_lower)
                    || p.tags.iter().any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect();

        Ok(results)
    }

    /// Get info about a specific plugin from the registry.
    pub async fn fetch_info(&self, name: &str) -> Result<Option<RegistryEntry>> {
        let index = self.fetch_index().await?;
        Ok(index.plugins.into_iter().find(|p| p.name == name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_registry_index() {
        let json = r#"{
            "plugins": [
                {
                    "name": "security",
                    "version": "1.0.0",
                    "description": "Security enforcement",
                    "author": "folktech",
                    "repo": "https://github.com/FolkTechAI/ftai-security",
                    "tags": ["security", "testing"]
                }
            ]
        }"#;

        let index: RegistryIndex = serde_json::from_str(json).unwrap();
        assert_eq!(index.plugins.len(), 1);
        assert_eq!(index.plugins[0].name, "security");
        assert_eq!(index.plugins[0].tags, vec!["security", "testing"]);
    }

    #[test]
    fn test_registry_client_default_url() {
        let client = RegistryClient::new(None);
        assert!(client.url.contains("ftai-registry"));
    }

    #[test]
    fn test_registry_client_custom_url() {
        let client = RegistryClient::new(Some("https://example.com/registry.json"));
        assert_eq!(client.url, "https://example.com/registry.json");
    }
}
