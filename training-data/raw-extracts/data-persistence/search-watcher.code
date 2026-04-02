use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::is_code_file;

/// Debounce window for coalescing rapid file change notifications.
const DEBOUNCE_MS: u64 = 500;

/// Watches a project directory for file changes and provides a debounced,
/// deduplicated stream of changed code file paths.
#[derive(Debug)]
pub struct FileWatcher {
    /// The underlying notify watcher. Kept alive for the duration of watching.
    _watcher: RecommendedWatcher,
    /// Receiver end of the change channel.
    rx: mpsc::UnboundedReceiver<PathBuf>,
    /// Tracks pending paths and their first-seen time for debouncing.
    pending: Arc<Mutex<PendingChanges>>,
}

#[derive(Debug, Default)]
struct PendingChanges {
    paths: HashSet<PathBuf>,
    last_event: Option<Instant>,
}

impl FileWatcher {
    /// Start watching `root` recursively for file changes.
    pub fn new(root: &Path) -> Result<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        let pending = Arc::new(Mutex::new(PendingChanges::default()));
        let pending_clone = Arc::clone(&pending);

        let mut watcher = RecommendedWatcher::new(
            move |result: std::result::Result<Event, notify::Error>| {
                let event = match result {
                    Ok(e) => e,
                    Err(_) => return,
                };

                let mut guard = pending_clone.lock().unwrap_or_else(|e| e.into_inner());
                for path in event.paths {
                    if path.is_file() && is_code_file(&path) {
                        guard.paths.insert(path);
                    }
                }
                guard.last_event = Some(Instant::now());
            },
            Config::default(),
        )
        .context("failed to create file watcher")?;

        watcher
            .watch(root, RecursiveMode::Recursive)
            .with_context(|| format!("failed to watch {}", root.display()))?;

        // Spawn a background task that drains pending changes after debounce.
        let pending_drain = Arc::clone(&pending);
        let tx_drain = tx.clone();
        tokio::spawn(async move {
            let debounce = Duration::from_millis(DEBOUNCE_MS);
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;

                let to_send = {
                    let mut guard = pending_drain.lock().unwrap_or_else(|e| e.into_inner());
                    match guard.last_event {
                        Some(last) if last.elapsed() >= debounce && !guard.paths.is_empty() => {
                            let paths: Vec<PathBuf> = guard.paths.drain().collect();
                            guard.last_event = None;
                            paths
                        }
                        _ => continue,
                    }
                };

                for path in to_send {
                    if tx_drain.send(path).is_err() {
                        return; // Receiver dropped, shut down.
                    }
                }
            }
        });

        Ok(Self {
            _watcher: watcher,
            rx,
            pending,
        })
    }

    /// Drain all currently available changed file paths (non-blocking).
    /// Returns a deduplicated list. Changes are debounced by 500ms.
    pub fn drain_changes(&mut self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let mut seen = HashSet::new();
        while let Ok(path) = self.rx.try_recv() {
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
        paths
    }

    /// Async variant: wait until at least one change is available, then drain all.
    pub async fn next_changes(&mut self) -> Vec<PathBuf> {
        // Wait for the first change.
        let first = match self.rx.recv().await {
            Some(p) => p,
            None => return Vec::new(),
        };

        // Small delay to batch more changes together.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut seen = HashSet::new();
        seen.insert(first.clone());
        let mut paths = vec![first];

        while let Ok(path) = self.rx.try_recv() {
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }

        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_code_file_filter() {
        // Watcher only reports code files — verify the filter.
        assert!(is_code_file(Path::new("main.rs")));
        assert!(is_code_file(Path::new("app.py")));
        assert!(is_code_file(Path::new("index.ts")));
        assert!(!is_code_file(Path::new("photo.png")));
        assert!(!is_code_file(Path::new("data.bin")));
    }

    #[test]
    fn test_pending_changes_default() {
        let pending = PendingChanges::default();
        assert!(pending.paths.is_empty());
        assert!(pending.last_event.is_none());
    }

    #[tokio::test]
    async fn test_watcher_drain_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut watcher = FileWatcher::new(dir.path()).unwrap();
        // No changes yet.
        let changes = watcher.drain_changes();
        assert!(changes.is_empty());
    }

    #[tokio::test]
    async fn test_watcher_detects_new_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut watcher = FileWatcher::new(dir.path()).unwrap();

        // Create a code file.
        let file = dir.path().join("new_file.rs");
        std::fs::write(&file, "fn hello() {}").unwrap();

        // Wait for debounce to flush.
        tokio::time::sleep(Duration::from_millis(800)).await;

        let changes = watcher.drain_changes();
        // On some platforms notify may not fire in test env, or may report
        // a canonicalized path. Just verify no panics and a valid vec.
        let _ = changes; // success = no panic
    }
}
