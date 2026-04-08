use anyhow::{Context, Result};
use std::path::Path;

use super::analyzer::{OutcomeType, SessionOutcome, ToolCallRecord, ToolResultType, UserFeedback};
use super::generator::GeneratedRule;

/// SQLite-backed storage for evolution data (session outcomes, generated rules).
#[derive(Debug)]
pub struct EvolutionStore {
    conn: rusqlite::Connection,
}

impl EvolutionStore {
    /// Open (or create) the evolution database at the given path. Uses WAL mode.
    pub fn open(db_path: &Path) -> Result<Self> {
        let conn =
            rusqlite::Connection::open(db_path).context("Failed to open evolution database")?;

        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id              TEXT PRIMARY KEY,
                project         TEXT NOT NULL,
                timestamp       INTEGER NOT NULL,
                task_description TEXT NOT NULL,
                outcome         TEXT NOT NULL,
                user_feedback   TEXT,
                total_tokens    INTEGER NOT NULL DEFAULT 0,
                retries         INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS tool_calls (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id        TEXT NOT NULL REFERENCES sessions(id),
                seq               INTEGER NOT NULL,
                tool_name         TEXT NOT NULL,
                arguments_summary TEXT NOT NULL,
                result_type       TEXT NOT NULL,
                error_message     TEXT,
                duration_ms       INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_tool_calls_session ON tool_calls(session_id);

            CREATE TABLE IF NOT EXISTS generated_rules (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                name                TEXT NOT NULL UNIQUE,
                source              TEXT NOT NULL DEFAULT 'pattern',
                confidence          REAL NOT NULL DEFAULT 0.0,
                ftai_rule           TEXT NOT NULL,
                generated_at        INTEGER NOT NULL,
                applied_count       INTEGER NOT NULL DEFAULT 0,
                success_after_apply INTEGER NOT NULL DEFAULT 0,
                disabled            INTEGER NOT NULL DEFAULT 0
            );
            ",
        )
        .context("Failed to initialize evolution schema")?;

        Ok(Self { conn })
    }

    /// Save a complete session outcome and its tool calls.
    pub fn save_outcome(&self, outcome: &SessionOutcome) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        tx.execute(
            "INSERT OR REPLACE INTO sessions (id, project, timestamp, task_description, outcome, user_feedback, total_tokens, retries)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                outcome.session_id,
                outcome.project,
                outcome.timestamp,
                outcome.task_description,
                outcome.success.to_db_string(),
                outcome.user_feedback.as_ref().map(|f| f.to_db_string()),
                outcome.total_tokens,
                outcome.retries,
            ],
        )?;

        // Clear existing tool calls for this session (idempotent on re-save).
        tx.execute(
            "DELETE FROM tool_calls WHERE session_id = ?1",
            [&outcome.session_id],
        )?;

        for (seq, tc) in outcome.tool_calls.iter().enumerate() {
            let error_message = match &tc.result_type {
                ToolResultType::Error(msg) => Some(msg.as_str()),
                _ => None,
            };
            tx.execute(
                "INSERT INTO tool_calls (session_id, seq, tool_name, arguments_summary, result_type, error_message, duration_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    outcome.session_id,
                    seq as i64,
                    tc.tool_name,
                    tc.arguments_summary,
                    tc.result_type.to_db_string(),
                    error_message,
                    tc.duration_ms,
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Load the most recent sessions, up to `limit`.
    pub fn recent_sessions(&self, limit: usize) -> Result<Vec<SessionOutcome>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project, timestamp, task_description, outcome, user_feedback, total_tokens, retries
             FROM sessions ORDER BY timestamp DESC LIMIT ?1",
        )?;

        let session_rows: Vec<(
            String,
            String,
            i64,
            String,
            String,
            Option<String>,
            usize,
            usize,
        )> = stmt
            .query_map([limit], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut results = Vec::with_capacity(session_rows.len());
        for (id, project, timestamp, task_desc, outcome_str, feedback_str, tokens, retries) in
            session_rows
        {
            let tool_calls = self.load_tool_calls(&id)?;
            results.push(SessionOutcome {
                session_id: id,
                project,
                timestamp,
                task_description: task_desc,
                tool_calls,
                success: OutcomeType::from_db_string(&outcome_str),
                user_feedback: feedback_str.map(|s| UserFeedback::from_db_string(&s)),
                total_tokens: tokens,
                retries,
            });
        }
        Ok(results)
    }

    fn load_tool_calls(&self, session_id: &str) -> Result<Vec<ToolCallRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT tool_name, arguments_summary, result_type, duration_ms
             FROM tool_calls WHERE session_id = ?1 ORDER BY seq",
        )?;
        let records = stmt
            .query_map([session_id], |row| {
                let result_type_str: String = row.get(2)?;
                Ok(ToolCallRecord {
                    tool_name: row.get(0)?,
                    arguments_summary: row.get(1)?,
                    result_type: ToolResultType::from_db_string(&result_type_str),
                    duration_ms: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(records)
    }

    /// Count total sessions stored.
    pub fn session_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
        Ok(count as usize)
    }

    /// Save a generated rule.
    pub fn save_generated_rule(&self, rule: &GeneratedRule) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        self.conn.execute(
            "INSERT OR REPLACE INTO generated_rules (name, source, confidence, ftai_rule, generated_at, applied_count, success_after_apply, disabled)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 0)",
            rusqlite::params![
                rule.name,
                rule.source.to_db_string(),
                rule.confidence,
                rule.ftai_rule,
                now,
            ],
        )?;
        Ok(())
    }

    /// Return active rules: confidence >= 0.7, not disabled, max 20.
    pub fn active_rules(&self) -> Result<Vec<GeneratedRule>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, source, confidence, ftai_rule
             FROM generated_rules
             WHERE confidence >= 0.7 AND disabled = 0
             ORDER BY confidence DESC
             LIMIT 20",
        )?;
        let rules = stmt
            .query_map([], |row| {
                let source_str: String = row.get(1)?;
                Ok(GeneratedRule {
                    name: row.get(0)?,
                    source: super::generator::RuleSource::from_db_string(&source_str),
                    confidence: row.get(2)?,
                    ftai_rule: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rules)
    }

    /// Increment the applied_count for a rule by name.
    /// Used when an evolution-generated rule is applied during a session.
    #[allow(dead_code)]
    pub fn increment_rule_applied(&self, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE generated_rules SET applied_count = applied_count + 1 WHERE name = ?1",
            [name],
        )?;
        Ok(())
    }

    /// Increment the success_after_apply counter for a rule.
    /// Used when a session succeeds after an evolution rule was applied.
    #[allow(dead_code)]
    pub fn increment_rule_success(&self, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE generated_rules SET success_after_apply = success_after_apply + 1 WHERE name = ?1",
            [name],
        )?;
        Ok(())
    }

    /// Disable a rule by name.
    pub fn disable_rule(&self, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE generated_rules SET disabled = 1 WHERE name = ?1",
            [name],
        )?;
        Ok(())
    }

    /// Check if a rule should be auto-disabled: applied 3+ times with 0 successes.
    pub fn check_auto_disable(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT name FROM generated_rules
             WHERE disabled = 0 AND applied_count >= 3 AND success_after_apply = 0",
        )?;
        let names = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolution::analyzer::*;
    use crate::evolution::generator::*;
    use tempfile::NamedTempFile;

    fn test_store() -> EvolutionStore {
        let tmp = NamedTempFile::new().expect("tmp file");
        EvolutionStore::open(tmp.path()).expect("open store")
    }

    fn sample_outcome(id: &str, project: &str, outcome: OutcomeType) -> SessionOutcome {
        SessionOutcome {
            session_id: id.to_string(),
            project: project.to_string(),
            timestamp: 1711500000,
            task_description: "Test task".to_string(),
            tool_calls: vec![
                ToolCallRecord {
                    tool_name: "file_read".to_string(),
                    arguments_summary: "path=/src/lib.rs".to_string(),
                    result_type: ToolResultType::Success,
                    duration_ms: 5,
                },
                ToolCallRecord {
                    tool_name: "file_edit".to_string(),
                    arguments_summary: "path=/src/lib.rs".to_string(),
                    result_type: ToolResultType::Success,
                    duration_ms: 20,
                },
            ],
            success: outcome,
            user_feedback: Some(UserFeedback::Accepted),
            total_tokens: 3000,
            retries: 0,
        }
    }

    #[test]
    fn test_outcome_storage_roundtrip() {
        let store = test_store();
        let outcome = sample_outcome("s1", "ftai", OutcomeType::Success);
        store.save_outcome(&outcome).expect("save");

        assert_eq!(store.session_count().unwrap(), 1);

        let loaded = store.recent_sessions(10).unwrap();
        assert_eq!(loaded.len(), 1);
        let s = &loaded[0];
        assert_eq!(s.session_id, "s1");
        assert_eq!(s.project, "ftai");
        assert_eq!(s.tool_calls.len(), 2);
        assert_eq!(s.tool_calls[0].tool_name, "file_read");
        assert_eq!(s.tool_calls[1].tool_name, "file_edit");
        assert_eq!(s.success, OutcomeType::Success);
        assert_eq!(s.user_feedback, Some(UserFeedback::Accepted));
        assert_eq!(s.total_tokens, 3000);
    }

    #[test]
    fn test_outcome_with_error_tool_call() {
        let store = test_store();
        let mut outcome = sample_outcome("s2", "ftai", OutcomeType::Failure("compile".into()));
        outcome.tool_calls.push(ToolCallRecord {
            tool_name: "bash".to_string(),
            arguments_summary: "cargo build".to_string(),
            result_type: ToolResultType::Error("exit code 1".to_string()),
            duration_ms: 5000,
        });
        store.save_outcome(&outcome).unwrap();

        let loaded = store.recent_sessions(10).unwrap();
        assert_eq!(loaded[0].tool_calls.len(), 3);
        assert_eq!(
            loaded[0].tool_calls[2].result_type,
            ToolResultType::Error("exit code 1".to_string())
        );
    }

    #[test]
    fn test_generated_rule_storage() {
        let store = test_store();
        let rule = GeneratedRule {
            name: "read-before-edit".to_string(),
            source: RuleSource::OrderingPattern,
            confidence: 0.85,
            ftai_rule:
                "rule read_before_edit {\n  scope \"*\"\n  require file_read before file_edit\n}"
                    .to_string(),
        };
        store.save_generated_rule(&rule).unwrap();

        let active = store.active_rules().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "read-before-edit");
        assert!((active[0].confidence - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn test_low_confidence_rule_excluded() {
        let store = test_store();
        let rule = GeneratedRule {
            name: "weak-pattern".to_string(),
            source: RuleSource::RepeatedFailure,
            confidence: 0.5,
            ftai_rule: "rule weak { }".to_string(),
        };
        store.save_generated_rule(&rule).unwrap();

        let active = store.active_rules().unwrap();
        assert!(active.is_empty());
    }

    #[test]
    fn test_rule_disable() {
        let store = test_store();
        let rule = GeneratedRule {
            name: "to-disable".to_string(),
            source: RuleSource::OrderingPattern,
            confidence: 0.9,
            ftai_rule: "rule td { }".to_string(),
        };
        store.save_generated_rule(&rule).unwrap();
        assert_eq!(store.active_rules().unwrap().len(), 1);

        store.disable_rule("to-disable").unwrap();
        assert!(store.active_rules().unwrap().is_empty());
    }

    #[test]
    fn test_auto_disable_check() {
        let store = test_store();
        let rule = GeneratedRule {
            name: "bad-rule".to_string(),
            source: RuleSource::OrderingPattern,
            confidence: 0.8,
            ftai_rule: "rule br { }".to_string(),
        };
        store.save_generated_rule(&rule).unwrap();

        // Apply 3 times, never succeeds.
        for _ in 0..3 {
            store.increment_rule_applied("bad-rule").unwrap();
        }

        let to_disable = store.check_auto_disable().unwrap();
        assert_eq!(to_disable, vec!["bad-rule"]);
    }

    #[test]
    fn test_max_20_active_rules() {
        let store = test_store();
        for i in 0..25 {
            let rule = GeneratedRule {
                name: format!("rule-{i}"),
                source: RuleSource::ProjectPattern,
                confidence: 0.8 + (i as f64) * 0.001,
                ftai_rule: format!("rule r{i} {{ }}"),
            };
            store.save_generated_rule(&rule).unwrap();
        }
        let active = store.active_rules().unwrap();
        assert_eq!(active.len(), 20);
    }
}
