use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// RAII lock guard — removes lock file on drop.
#[allow(dead_code)]
pub struct DreamLock {
    path: PathBuf,
}

impl DreamLock {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for DreamLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Summary metadata for a single dream file.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DreamSummary {
    pub filename: String,
    pub modified: u64,
}

/// Manages dream scheduling: time gates, session count gates, and lock files.
pub struct DreamScheduler {
    dream_dir: PathBuf,
    lock_path: PathBuf,
}

#[allow(dead_code)]
impl DreamScheduler {
    pub fn new(project_path: &Path) -> Self {
        let dream_dir = project_path.join(".ftai").join("dreams");
        let lock_path = dream_dir.join(".lock");
        Self {
            dream_dir,
            lock_path,
        }
    }

    /// Check all three gates:
    /// 1. >= 24 hours since last dream
    /// 2. >= 3 session transcripts since last dream
    /// 3. No concurrent dream (lock file absent)
    pub fn should_dream(&self, transcripts_dir: &Path) -> bool {
        // Gate 3: lock must not exist
        if self.lock_path.exists() {
            return false;
        }

        // Gate 1: >= 24 hours since last dream
        let last = self.last_dream_time();
        let now = now_secs();
        if let Some(last_time) = last {
            if now.saturating_sub(last_time) < 24 * 60 * 60 {
                return false;
            }
        }
        // If no last dream time, gate 1 passes (never dreamed).

        // Gate 2: >= 3 transcripts since last dream
        let count = self.session_count_since_last_dream(transcripts_dir);
        if count < 3 {
            return false;
        }

        true
    }

    /// Returns the modification time (unix epoch seconds) of the most recent dream file.
    pub fn last_dream_time(&self) -> Option<u64> {
        if !self.dream_dir.exists() {
            return None;
        }
        let mut latest: Option<u64> = None;
        if let Ok(entries) = std::fs::read_dir(&self.dream_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                // Skip lock file and non-md files
                if path.extension().map_or(true, |e| e != "md") {
                    continue;
                }
                if let Ok(meta) = path.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        if let Ok(dur) = mtime.duration_since(UNIX_EPOCH) {
                            let secs = dur.as_secs();
                            latest = Some(latest.map_or(secs, |prev: u64| prev.max(secs)));
                        }
                    }
                }
            }
        }
        latest
    }

    /// Count .jsonl transcript files newer than the last dream.
    pub fn session_count_since_last_dream(&self, transcripts_dir: &Path) -> usize {
        let cutoff = self.last_dream_time().unwrap_or(0);
        if !transcripts_dir.exists() {
            return 0;
        }
        let mut count = 0;
        if let Ok(entries) = std::fs::read_dir(transcripts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(true, |e| e != "jsonl") {
                    continue;
                }
                if let Ok(meta) = path.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        if let Ok(dur) = mtime.duration_since(UNIX_EPOCH) {
                            if dur.as_secs() > cutoff {
                                count += 1;
                            }
                        }
                    }
                }
            }
        }
        count
    }

    /// Acquire the dream lock. Returns a RAII guard that removes the lock on drop.
    pub fn acquire_lock(&self) -> std::io::Result<DreamLock> {
        std::fs::create_dir_all(&self.dream_dir)?;

        // Fail if lock already exists (prevents concurrent dreams)
        if self.lock_path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Dream lock already held",
            ));
        }

        std::fs::write(&self.lock_path, format!("{}", std::process::id()))?;
        Ok(DreamLock::new(self.lock_path.clone()))
    }

    /// List all dream files with metadata.
    pub fn list_dreams(&self) -> Vec<DreamSummary> {
        let mut dreams = Vec::new();
        if !self.dream_dir.exists() {
            return dreams;
        }
        if let Ok(entries) = std::fs::read_dir(&self.dream_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(true, |e| e != "md") {
                    continue;
                }
                let filename = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let modified = path
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                dreams.push(DreamSummary { filename, modified });
            }
        }
        dreams.sort_by(|a, b| b.modified.cmp(&a.modified));
        dreams
    }

    /// Read the content of .ftai/dreams/latest.md if it exists.
    pub fn latest_dream(&self) -> Option<String> {
        let path = self.dream_dir.join("latest.md");
        std::fs::read_to_string(path).ok()
    }
}

#[allow(dead_code)]
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::thread;
    use std::time::Duration;

    fn setup_project(tmp: &TempDir) -> (PathBuf, PathBuf) {
        let project = tmp.path().to_path_buf();
        let transcripts = project.join(".ftai").join("transcripts");
        std::fs::create_dir_all(&transcripts).unwrap();
        (project, transcripts)
    }

    fn create_transcript(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), "{\"role\":\"user\",\"content\":\"test\"}\n").unwrap();
    }

    fn create_dream_file(dream_dir: &Path, name: &str) {
        std::fs::create_dir_all(dream_dir).unwrap();
        std::fs::write(dream_dir.join(name), "# Dream\n").unwrap();
    }

    // ── should_dream tests ────────────────────────────────────────────────

    #[test]
    fn test_should_dream_false_no_transcripts() {
        let tmp = TempDir::new().unwrap();
        let (project, transcripts) = setup_project(&tmp);
        let scheduler = DreamScheduler::new(&project);
        assert!(!scheduler.should_dream(&transcripts));
    }

    #[test]
    fn test_should_dream_false_dreamed_recently() {
        let tmp = TempDir::new().unwrap();
        let (project, transcripts) = setup_project(&tmp);

        // Create a dream file (recent — its mtime is now)
        let dream_dir = project.join(".ftai").join("dreams");
        create_dream_file(&dream_dir, "recent-dream.md");

        // Create 5 transcripts
        for i in 0..5 {
            create_transcript(&transcripts, &format!("{}.jsonl", 1000 + i));
        }

        let scheduler = DreamScheduler::new(&project);
        // Dream file is fresh (<24h), so should_dream returns false
        assert!(!scheduler.should_dream(&transcripts));
    }

    #[test]
    fn test_should_dream_true_when_conditions_met() {
        let tmp = TempDir::new().unwrap();
        let (project, transcripts) = setup_project(&tmp);

        // No dream files exist, 3+ transcripts present
        for i in 0..4 {
            create_transcript(&transcripts, &format!("{}.jsonl", 2000 + i));
        }

        let scheduler = DreamScheduler::new(&project);
        // No previous dream => time gate passes, 4 transcripts >= 3 => count gate passes
        assert!(scheduler.should_dream(&transcripts));
    }

    // ── Lock tests ────────────────────────────────────────────────────────

    #[test]
    fn test_lock_prevents_concurrent_dreams() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().to_path_buf();
        let scheduler = DreamScheduler::new(&project);

        let _lock = scheduler.acquire_lock().unwrap();
        // Second acquire should fail
        let result = scheduler.acquire_lock();
        assert!(result.is_err());
    }

    #[test]
    fn test_lock_released_on_drop() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().to_path_buf();
        let scheduler = DreamScheduler::new(&project);

        {
            let _lock = scheduler.acquire_lock().unwrap();
            assert!(scheduler.lock_path.exists());
        }
        // Lock file should be gone after drop
        assert!(!scheduler.lock_path.exists());
    }

    // ── List dreams ───────────────────────────────────────────────────────

    #[test]
    fn test_list_dreams_empty() {
        let tmp = TempDir::new().unwrap();
        let scheduler = DreamScheduler::new(tmp.path());
        assert!(scheduler.list_dreams().is_empty());
    }

    #[test]
    fn test_list_dreams_returns_md_files() {
        let tmp = TempDir::new().unwrap();
        let dream_dir = tmp.path().join(".ftai").join("dreams");
        create_dream_file(&dream_dir, "2026-03-29-auth.md");
        // Small sleep to ensure different mtimes
        thread::sleep(Duration::from_millis(10));
        create_dream_file(&dream_dir, "latest.md");
        // Non-md file should be ignored
        std::fs::write(dream_dir.join(".lock"), "pid").unwrap();

        let scheduler = DreamScheduler::new(tmp.path());
        let dreams = scheduler.list_dreams();
        assert_eq!(dreams.len(), 2);
        // Sorted by modified desc — latest should be first
        assert_eq!(dreams[0].filename, "latest.md");
    }

    #[test]
    fn test_latest_dream_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        let scheduler = DreamScheduler::new(tmp.path());
        assert!(scheduler.latest_dream().is_none());
    }

    #[test]
    fn test_latest_dream_reads_content() {
        let tmp = TempDir::new().unwrap();
        let dream_dir = tmp.path().join(".ftai").join("dreams");
        std::fs::create_dir_all(&dream_dir).unwrap();
        std::fs::write(dream_dir.join("latest.md"), "# Dream Summary\nContent here").unwrap();

        let scheduler = DreamScheduler::new(tmp.path());
        let content = scheduler.latest_dream().unwrap();
        assert!(content.contains("Dream Summary"));
    }

    // ── P0 Security Red Tests ─────────────────────────────────────────────

    #[test]
    fn test_p0_lock_path_inside_ftai() {
        let tmp = TempDir::new().unwrap();
        let scheduler = DreamScheduler::new(tmp.path());
        // Lock path must be inside .ftai/dreams/
        let expected_prefix = tmp.path().join(".ftai").join("dreams");
        assert!(scheduler.lock_path.starts_with(&expected_prefix));
    }
}
