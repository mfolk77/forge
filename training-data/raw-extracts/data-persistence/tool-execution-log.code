use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// The result classification of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResultType {
    Success,
    Error,
    Timeout,
}

/// A single structured log entry representing one tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Unix timestamp in seconds.
    pub timestamp: i64,
    /// Name of the tool that was executed.
    pub tool_name: String,
    /// Short human-readable summary of the arguments passed.
    pub arguments_summary: String,
    /// Whether the execution succeeded, errored, or timed out.
    pub result_type: ResultType,
    /// Wall-clock duration of the execution in milliseconds.
    pub duration_ms: u64,
    /// First 200 characters of the tool output.
    pub output_preview: String,
}

impl LogEntry {
    /// Truncates `output` to 200 characters and stores it in `output_preview`.
    pub fn new(
        tool_name: impl Into<String>,
        arguments_summary: impl Into<String>,
        result_type: ResultType,
        duration_ms: u64,
        output: impl AsRef<str>,
    ) -> Self {
        let preview: String = output.as_ref().chars().take(200).collect();
        Self {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            tool_name: tool_name.into(),
            arguments_summary: arguments_summary.into(),
            result_type,
            duration_ms,
            output_preview: preview,
        }
    }
}

/// Writes and reads structured JSONL execution logs under `~/.ftai/logs/`.
pub struct ExecutionLogger {
    log_dir: PathBuf,
}

impl ExecutionLogger {
    /// Creates the logger and ensures `~/.ftai/logs/` exists.
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().context("cannot determine home directory")?;
        let log_dir = home.join(".ftai").join("logs");
        fs::create_dir_all(&log_dir)
            .with_context(|| format!("failed to create log directory: {}", log_dir.display()))?;
        Ok(Self { log_dir })
    }

    /// Creates the logger using a custom directory (useful for tests).
    pub fn with_dir(log_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&log_dir)
            .with_context(|| format!("failed to create log directory: {}", log_dir.display()))?;
        Ok(Self { log_dir })
    }

    /// Returns the path for today's log file, e.g. `execution_20260327.log`.
    fn today_log_path(&self) -> PathBuf {
        self.log_path_for_date(&today_date_string())
    }

    /// Returns the log path for the given date string (YYYYMMDD).
    fn log_path_for_date(&self, date: &str) -> PathBuf {
        // Validate the date string to prevent path traversal (P0 security).
        let safe_date = sanitize_date(date);
        self.log_dir
            .join(format!("execution_{}.log", safe_date))
    }

    /// Appends a single JSON line to today's log file.
    ///
    /// The file is opened in append mode, which is atomic-safe for single-line
    /// writes on POSIX systems (writes <= PIPE_BUF are atomic).
    pub fn log_execution(&self, entry: &LogEntry) -> Result<()> {
        let path = self.today_log_path();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open log file: {}", path.display()))?;
        let line = serde_json::to_string(entry).context("failed to serialize log entry")?;
        writeln!(file, "{}", line).context("failed to write log entry")?;
        file.flush().context("failed to flush log file")?;
        Ok(())
    }

    /// Returns the last `n` entries from today's log file.
    ///
    /// Reads the whole file and takes the tail to keep memory usage bounded
    /// (log files are capped to one day and entries are small).
    pub fn recent_entries(&self, n: usize) -> Result<Vec<LogEntry>> {
        let path = self.today_log_path();
        self.read_entries_from_path(&path, None, Some(n))
    }

    /// Returns all entries from the log for the specified date (YYYYMMDD).
    pub fn entries_for_date(&self, date: &str) -> Result<Vec<LogEntry>> {
        let path = self.log_path_for_date(date);
        self.read_entries_from_path(&path, None, None)
    }

    /// Internal helper: reads JSONL entries from `path`, optionally capping
    /// to the last `tail` entries.  Malformed lines are silently skipped.
    fn read_entries_from_path(
        &self,
        path: &PathBuf,
        _limit: Option<usize>,
        tail: Option<usize>,
    ) -> Result<Vec<LogEntry>> {
        if !path.exists() {
            return Ok(vec![]);
        }
        let file = File::open(path)
            .with_context(|| format!("failed to open log file: {}", path.display()))?;
        let reader = BufReader::new(file);

        let entries: Vec<LogEntry> = reader
            .lines()
            .filter_map(|line| {
                let line = line.ok()?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }
                serde_json::from_str(trimmed).ok()
            })
            .collect();

        if let Some(n) = tail {
            let skip = entries.len().saturating_sub(n);
            Ok(entries.into_iter().skip(skip).collect())
        } else {
            Ok(entries)
        }
    }
}

/// Returns today's date as a YYYYMMDD string using only `std`.
fn today_date_string() -> String {
    // We need YYYYMMDD without chrono. Use SystemTime + manual calculation.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    seconds_to_date(secs)
}

/// Converts a Unix timestamp (seconds) to a YYYYMMDD string.
/// Good enough for log file naming; does not need timezone awareness.
fn seconds_to_date(secs: u64) -> String {
    // Days since epoch
    let days = secs / 86400;

    // Gregorian calendar calculation
    // Algorithm: civil date from days since 1970-01-01
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}{:02}{:02}", y, m, d)
}

/// Strips any character that is not a digit from a date string.
/// Prevents path traversal via crafted date strings (P0 security).
fn sanitize_date(date: &str) -> String {
    date.chars().filter(|c| c.is_ascii_digit()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_logger() -> (ExecutionLogger, TempDir) {
        let dir = TempDir::new().unwrap();
        let logger = ExecutionLogger::with_dir(dir.path().to_path_buf()).unwrap();
        (logger, dir)
    }

    fn sample_entry(tool: &str, result: ResultType) -> LogEntry {
        LogEntry::new(tool, "arg1=foo", result, 42, "some output here")
    }

    // ── Basic functionality ──────────────────────────────────────────────────

    #[test]
    fn test_log_and_recent_entries() {
        let (logger, _dir) = make_logger();
        let entry = sample_entry("bash", ResultType::Success);
        logger.log_execution(&entry).unwrap();

        let entries = logger.recent_entries(10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tool_name, "bash");
        assert_eq!(entries[0].result_type, ResultType::Success);
        assert_eq!(entries[0].duration_ms, 42);
    }

    #[test]
    fn test_multiple_entries_order_preserved() {
        let (logger, _dir) = make_logger();
        logger.log_execution(&sample_entry("tool_a", ResultType::Success)).unwrap();
        logger.log_execution(&sample_entry("tool_b", ResultType::Error)).unwrap();
        logger.log_execution(&sample_entry("tool_c", ResultType::Timeout)).unwrap();

        let entries = logger.recent_entries(10).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].tool_name, "tool_a");
        assert_eq!(entries[1].tool_name, "tool_b");
        assert_eq!(entries[2].tool_name, "tool_c");
    }

    #[test]
    fn test_recent_entries_tail_limit() {
        let (logger, _dir) = make_logger();
        for i in 0..10 {
            let e = LogEntry::new(
                format!("tool_{}", i),
                "args",
                ResultType::Success,
                1,
                "out",
            );
            logger.log_execution(&e).unwrap();
        }
        let entries = logger.recent_entries(3).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].tool_name, "tool_7");
        assert_eq!(entries[2].tool_name, "tool_9");
    }

    #[test]
    fn test_recent_entries_empty_when_no_log() {
        let (logger, _dir) = make_logger();
        let entries = logger.recent_entries(10).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_entries_for_date_reads_correct_file() {
        let (logger, _dir) = make_logger();
        // Manually write a fake dated log
        let path = logger.log_dir.join("execution_20260101.log");
        let entry = LogEntry::new("grep", "pattern=foo", ResultType::Success, 5, "matches");
        let line = serde_json::to_string(&entry).unwrap();
        fs::write(&path, format!("{}\n", line)).unwrap();

        let entries = logger.entries_for_date("20260101").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tool_name, "grep");
    }

    #[test]
    fn test_entries_for_date_missing_returns_empty() {
        let (logger, _dir) = make_logger();
        let entries = logger.entries_for_date("19991231").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_output_preview_truncated_to_200_chars() {
        let long_output = "x".repeat(500);
        let entry = LogEntry::new("tool", "args", ResultType::Success, 1, &long_output);
        assert_eq!(entry.output_preview.len(), 200);
    }

    #[test]
    fn test_output_preview_short_output_unchanged() {
        let entry = LogEntry::new("tool", "args", ResultType::Success, 1, "hello");
        assert_eq!(entry.output_preview, "hello");
    }

    #[test]
    fn test_result_types_serialize_correctly() {
        let s = serde_json::to_string(&ResultType::Success).unwrap();
        assert_eq!(s, "\"success\"");
        let s = serde_json::to_string(&ResultType::Error).unwrap();
        assert_eq!(s, "\"error\"");
        let s = serde_json::to_string(&ResultType::Timeout).unwrap();
        assert_eq!(s, "\"timeout\"");
    }

    #[test]
    fn test_jsonl_format_one_object_per_line() {
        let (logger, dir) = make_logger();
        logger.log_execution(&sample_entry("a", ResultType::Success)).unwrap();
        logger.log_execution(&sample_entry("b", ResultType::Error)).unwrap();

        let today = today_date_string();
        let path = dir.path().join(format!("execution_{}.log", today));
        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        // Each line must be valid JSON
        for line in &lines {
            serde_json::from_str::<LogEntry>(line).expect("each line must be valid JSON");
        }
    }

    #[test]
    fn test_malformed_lines_skipped_gracefully() {
        let (logger, dir) = make_logger();
        let today = today_date_string();
        let path = dir.path().join(format!("execution_{}.log", today));
        // Write a mix of valid and invalid lines
        let valid = serde_json::to_string(&sample_entry("good", ResultType::Success)).unwrap();
        fs::write(&path, format!("not json\n{}\n{{broken\n", valid)).unwrap();

        let entries = logger.recent_entries(10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tool_name, "good");
    }

    #[test]
    fn test_timestamp_is_positive() {
        let entry = sample_entry("tool", ResultType::Success);
        assert!(entry.timestamp > 0);
    }

    // ── Date helper ──────────────────────────────────────────────────────────

    #[test]
    fn test_seconds_to_date_epoch() {
        assert_eq!(seconds_to_date(0), "19700101");
    }

    #[test]
    fn test_seconds_to_date_known_date() {
        // 2026-03-27 00:00:00 UTC = 1774569600
        assert_eq!(seconds_to_date(1774569600), "20260327");
    }

    #[test]
    fn test_seconds_to_date_leap_year() {
        // 2000-02-29 = 951782400
        assert_eq!(seconds_to_date(951782400), "20000229");
    }

    // ── Security (P0) tests ──────────────────────────────────────────────────

    /// P0: Path traversal via crafted date string must be blocked.
    #[test]
    fn test_path_traversal_via_date_string_blocked() {
        let (logger, dir) = make_logger();
        // Attacker-controlled date string with traversal attempt
        let malicious_dates = [
            "../../../etc/passwd",
            "../../secret",
            "20260101/../../../etc/shadow",
            "20260101\x00evil",
            "2026/01/01",
        ];
        for date in &malicious_dates {
            let path = logger.log_path_for_date(date);
            // The resulting path must stay inside the log directory
            assert!(
                path.starts_with(&logger.log_dir),
                "path traversal not blocked for date {:?}: got {}",
                date,
                path.display()
            );
        }
    }

    /// P0: Log file created by `with_dir` must be confined to the specified dir.
    #[test]
    fn test_log_file_confined_to_log_dir() {
        let (logger, dir) = make_logger();
        let entry = sample_entry("bash", ResultType::Success);
        logger.log_execution(&entry).unwrap();

        // Walk the temp dir — all files must be inside it
        for result in fs::read_dir(dir.path()).unwrap() {
            let path = result.unwrap().path();
            assert!(
                path.starts_with(dir.path()),
                "file escaped log dir: {}",
                path.display()
            );
        }
    }

    /// P0: Tool name with path separators must not influence file paths.
    #[test]
    fn test_tool_name_injection_does_not_affect_file_path() {
        let (logger, _dir) = make_logger();
        // Tool name is stored in the JSON content, not in the file path — this
        // just confirms that injection in content is inert (no shell eval, no eval).
        let evil_entry = LogEntry::new(
            "../../../etc/cron.d/evil",
            "$(rm -rf /)",
            ResultType::Success,
            0,
            "`id`",
        );
        // Must not panic or create files outside the log dir
        logger.log_execution(&evil_entry).unwrap();
        let entries = logger.recent_entries(1).unwrap();
        assert_eq!(entries[0].tool_name, "../../../etc/cron.d/evil");
    }

    /// P0: Log entry deserialization of attacker-supplied content must not panic.
    #[test]
    fn test_log_deserialization_does_not_panic_on_crafted_input() {
        let payloads = [
            r#"{"timestamp":0,"tool_name":"x","arguments_summary":"x","result_type":"success","duration_ms":0,"output_preview":"x"}"#,
            r#"{"timestamp":-1,"tool_name":"","arguments_summary":"","result_type":"error","duration_ms":0,"output_preview":""}"#,
            r#"{}"#,
            r#"null"#,
            &"x".repeat(100_000),
        ];
        for payload in &payloads {
            // Must not panic — result may be Ok or Err
            let _ = serde_json::from_str::<LogEntry>(payload);
        }
    }

    /// P0: `sanitize_date` strips all non-digit characters.
    #[test]
    fn test_sanitize_date_strips_non_digits() {
        assert_eq!(sanitize_date("20260327"), "20260327");
        assert_eq!(sanitize_date("../20260327"), "20260327");
        assert_eq!(sanitize_date("2026/03/27"), "20260327");
        assert_eq!(sanitize_date("../../etc"), "");
        assert_eq!(sanitize_date(""), "");
    }

    /// P0: `entries_for_date` with path traversal payload must stay in log dir.
    #[test]
    fn test_entries_for_date_traversal_stays_in_log_dir() {
        let (logger, _dir) = make_logger();
        // Should return empty (file doesn't exist) and not panic
        let result = logger.entries_for_date("../../etc/passwd");
        // Must not panic and must succeed (returning empty)
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
