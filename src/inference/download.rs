use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

const HF_BASE_URL: &str = "https://huggingface.co";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFormat {
    Gguf,
    Mlx,
}

#[derive(Debug)]
pub struct ModelDownloader {
    client: reqwest::Client,
}

impl ModelDownloader {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent("ftai/0.1")
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { client })
    }

    pub async fn download_model<F>(
        &self,
        model_name: &str,
        target_dir: &Path,
        progress: F,
    ) -> Result<PathBuf>
    where
        F: Fn(u64, u64) + Send + 'static,
    {
        validate_model_name(model_name)?;

        let format = detect_format(model_name);
        let model_dir = target_dir.join(sanitize_name(model_name));
        std::fs::create_dir_all(&model_dir)
            .with_context(|| format!("failed to create model directory: {}", model_dir.display()))?;

        match format {
            ModelFormat::Gguf => self.download_gguf(model_name, &model_dir, progress).await,
            ModelFormat::Mlx => self.download_mlx(model_name, &model_dir, progress).await,
        }
    }

    async fn download_gguf<F>(
        &self,
        model_name: &str,
        model_dir: &Path,
        progress: F,
    ) -> Result<PathBuf>
    where
        F: Fn(u64, u64) + Send + 'static,
    {
        let filename = gguf_filename(model_name);
        let url = format!(
            "{HF_BASE_URL}/{model_name}/resolve/main/{filename}"
        );
        let dest = model_dir.join(&filename);

        if dest.exists() {
            return Ok(dest);
        }

        self.download_file(&url, &dest, &progress).await?;
        Ok(dest)
    }

    async fn download_mlx<F>(
        &self,
        model_name: &str,
        model_dir: &Path,
        progress: F,
    ) -> Result<PathBuf>
    where
        F: Fn(u64, u64) + Send + 'static,
    {
        let required_files = [
            "config.json",
            "tokenizer.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
        ];

        for file in &required_files {
            let url = format!("{HF_BASE_URL}/{model_name}/resolve/main/{file}");
            let dest = model_dir.join(file);
            if !dest.exists() {
                self.download_file(&url, &dest, &progress).await?;
            }
        }

        // Download weight shards (model-00001-of-NNNNN.safetensors pattern)
        // Start with the weights index
        let index_url = format!(
            "{HF_BASE_URL}/{model_name}/resolve/main/model.safetensors.index.json"
        );
        let index_dest = model_dir.join("model.safetensors.index.json");

        if !index_dest.exists() {
            // Try single-file first
            let single_url = format!(
                "{HF_BASE_URL}/{model_name}/resolve/main/model.safetensors"
            );
            let single_dest = model_dir.join("model.safetensors");

            match self.download_file(&single_url, &single_dest, &progress).await {
                Ok(()) => return Ok(model_dir.to_path_buf()),
                Err(_) => {
                    self.download_file(&index_url, &index_dest, &progress).await?;
                }
            }
        }

        if index_dest.exists() {
            let index_content = std::fs::read_to_string(&index_dest)?;
            let index: serde_json::Value = serde_json::from_str(&index_content)?;

            if let Some(weight_map) = index.get("weight_map").and_then(|v| v.as_object()) {
                let mut shard_files: Vec<String> = weight_map
                    .values()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                shard_files.sort();
                shard_files.dedup();

                for shard in &shard_files {
                    // P0 FIX: Validate shard filenames from the weight index JSON.
                    // A malicious index could contain traversal paths like "../../etc/crontab"
                    // which would escape the model directory via model_dir.join(shard).
                    // OWASP: A08:2021 Software and Data Integrity Failures
                    if shard.contains("..") || shard.starts_with('/') || shard.starts_with('\\') || shard.contains('\0') {
                        bail!(
                            "refusing to download shard with suspicious filename: {:?} \
                             (path traversal, absolute path, or null byte detected)",
                            shard
                        );
                    }
                    let shard_url = format!("{HF_BASE_URL}/{model_name}/resolve/main/{shard}");
                    let shard_dest = model_dir.join(shard);
                    if !shard_dest.exists() {
                        self.download_file(&shard_url, &shard_dest, &progress).await?;
                    }
                }
            }
        }

        Ok(model_dir.to_path_buf())
    }

    async fn download_file<F>(&self, url: &str, dest: &Path, progress: &F) -> Result<()>
    where
        F: Fn(u64, u64),
    {
        use futures_util::StreamExt;

        let resp = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("failed to GET {url}"))?;

        if !resp.status().is_success() {
            bail!(
                "download failed: {} returned {}",
                url,
                resp.status()
            );
        }

        let total = resp.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let tmp_dest = dest.with_extension("part");
        let mut file = std::fs::File::create(&tmp_dest)
            .with_context(|| format!("failed to create {}", tmp_dest.display()))?;

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("error reading download stream")?;
            std::io::Write::write_all(&mut file, &chunk)?;
            downloaded += chunk.len() as u64;
            progress(downloaded, total);
        }

        drop(file);
        std::fs::rename(&tmp_dest, dest)
            .with_context(|| format!("failed to rename {} -> {}", tmp_dest.display(), dest.display()))?;

        Ok(())
    }

    pub fn is_model_cached(model_name: &str, models_dir: &Path) -> bool {
        let dir = models_dir.join(sanitize_name(model_name));
        if !dir.is_dir() {
            return false;
        }
        let format = detect_format(model_name);
        match format {
            ModelFormat::Gguf => {
                let filename = gguf_filename(model_name);
                dir.join(filename).exists()
            }
            ModelFormat::Mlx => {
                dir.join("config.json").exists()
                    && (dir.join("model.safetensors").exists()
                        || dir.join("model.safetensors.index.json").exists())
            }
        }
    }

    pub fn list_cached_models(models_dir: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(models_dir) else {
            return Vec::new();
        };

        entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    None
                } else {
                    Some(name)
                }
            })
            .collect()
    }
}

fn validate_model_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("model name cannot be empty");
    }
    if !name.contains('/') {
        bail!("model name must be in 'owner/repo' format, got: {name}");
    }
    if name.contains("..") {
        bail!("model name contains path traversal");
    }
    // Block absolute paths and other injection attempts
    if name.starts_with('/') || name.starts_with('\\') {
        bail!("model name must not be an absolute path");
    }
    if name.contains('\0') {
        bail!("model name contains null byte");
    }
    Ok(())
}

fn sanitize_name(model_name: &str) -> String {
    model_name.replace('/', "--")
}

fn detect_format(model_name: &str) -> ModelFormat {
    let lower = model_name.to_lowercase();
    if lower.contains("gguf") || lower.contains("-q4") || lower.contains("-q8") || lower.contains("-q5") || lower.contains("-q6") {
        ModelFormat::Gguf
    } else {
        ModelFormat::Mlx
    }
}

fn gguf_filename(model_name: &str) -> String {
    let repo = model_name.split('/').last().unwrap_or(model_name);
    format!("{repo}.gguf")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_validate_model_name_valid() {
        assert!(validate_model_name("TheBloke/Llama-2-7B-GGUF").is_ok());
        assert!(validate_model_name("mlx-community/Qwen-4bit").is_ok());
    }

    #[test]
    fn test_validate_model_name_empty() {
        assert!(validate_model_name("").is_err());
    }

    #[test]
    fn test_validate_model_name_no_slash() {
        assert!(validate_model_name("justarepo").is_err());
    }

    #[test]
    fn test_validate_model_name_path_traversal() {
        assert!(validate_model_name("../../../etc/passwd").is_err());
        assert!(validate_model_name("owner/..").is_err());
    }

    #[test]
    fn test_validate_model_name_absolute_path() {
        assert!(validate_model_name("/etc/passwd").is_err());
    }

    #[test]
    fn test_validate_model_name_null_byte() {
        assert!(validate_model_name("owner/repo\0injection").is_err());
    }

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name("TheBloke/Llama-2"), "TheBloke--Llama-2");
        assert_eq!(sanitize_name("org/model"), "org--model");
    }

    #[test]
    fn test_detect_format_gguf() {
        assert_eq!(detect_format("TheBloke/Llama-2-7B-GGUF"), ModelFormat::Gguf);
        assert_eq!(detect_format("owner/model-Q4_K_M"), ModelFormat::Gguf);
        assert_eq!(detect_format("owner/model-q8_0"), ModelFormat::Gguf);
    }

    #[test]
    fn test_detect_format_mlx() {
        assert_eq!(detect_format("mlx-community/Qwen-4bit"), ModelFormat::Mlx);
        assert_eq!(detect_format("owner/some-model"), ModelFormat::Mlx);
    }

    #[test]
    fn test_gguf_filename() {
        assert_eq!(gguf_filename("TheBloke/Llama-2-7B-GGUF"), "Llama-2-7B-GGUF.gguf");
    }

    #[test]
    fn test_list_cached_models_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let models = ModelDownloader::list_cached_models(tmp.path());
        assert!(models.is_empty());
    }

    #[test]
    fn test_list_cached_models_with_entries() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("model-a")).unwrap();
        fs::create_dir(tmp.path().join("model-b")).unwrap();
        fs::create_dir(tmp.path().join(".hidden")).unwrap();
        fs::write(tmp.path().join("not-a-dir.txt"), "").unwrap();

        let mut models = ModelDownloader::list_cached_models(tmp.path());
        models.sort();
        assert_eq!(models, vec!["model-a", "model-b"]);
    }

    #[test]
    fn test_list_cached_models_nonexistent_dir() {
        let models = ModelDownloader::list_cached_models(Path::new("/nonexistent/path"));
        assert!(models.is_empty());
    }

    #[test]
    fn test_is_model_cached_gguf() {
        let tmp = tempfile::tempdir().unwrap();
        let model_dir = tmp.path().join("TheBloke--Llama-GGUF");
        fs::create_dir(&model_dir).unwrap();
        fs::write(model_dir.join("Llama-GGUF.gguf"), b"fake").unwrap();

        assert!(ModelDownloader::is_model_cached(
            "TheBloke/Llama-GGUF",
            tmp.path()
        ));
    }

    #[test]
    fn test_is_model_cached_mlx() {
        let tmp = tempfile::tempdir().unwrap();
        let model_dir = tmp.path().join("mlx-community--Qwen-4bit");
        fs::create_dir(&model_dir).unwrap();
        fs::write(model_dir.join("config.json"), "{}").unwrap();
        fs::write(model_dir.join("model.safetensors"), b"fake").unwrap();

        assert!(ModelDownloader::is_model_cached(
            "mlx-community/Qwen-4bit",
            tmp.path()
        ));
    }

    #[test]
    fn test_is_model_cached_false() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!ModelDownloader::is_model_cached(
            "nonexistent/model",
            tmp.path()
        ));
    }
}
