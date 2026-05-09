use anyhow::{Context, Result};
use std::path::Path;

use crate::backend::types::{Message, Role, ToolCall};

/// Summary of a past session for listing.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub summary: Option<String>,
    pub message_count: i64,
}

/// Manages session persistence: messages, summaries, and continuity across restarts.
#[derive(Debug)]
pub struct SessionManager {
    conn: rusqlite::Connection,
    current_session_id: Option<String>,
    project: String,
}

impl SessionManager {
    /// Open (or create) the session database for a given project.
    pub fn open(db_path: &Path, project: &str) -> Result<Self> {
        let conn =
            rusqlite::Connection::open(db_path).context("Failed to open session database")?;

        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id            TEXT PRIMARY KEY,
                project       TEXT NOT NULL,
                started_at    INTEGER NOT NULL,
                ended_at      INTEGER,
                summary       TEXT,
                message_count INTEGER NOT NULL DEFAULT 0,
                total_tokens  INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS messages (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id      TEXT NOT NULL REFERENCES sessions(id),
                seq             INTEGER NOT NULL,
                role            TEXT NOT NULL,
                content         TEXT NOT NULL,
                tool_calls      TEXT,
                tool_call_id    TEXT,
                tokens_estimated INTEGER NOT NULL DEFAULT 0,
                timestamp       INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
            ",
        )
        .context("Failed to initialize session schema")?;

        Ok(Self {
            conn,
            current_session_id: None,
            project: project.to_string(),
        })
    }

    /// Start a new session. Returns the session ID.
    pub fn start_session(&mut self) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_epoch();

        self.conn.execute(
            "INSERT INTO sessions (id, project, started_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, self.project, now],
        )?;

        self.current_session_id = Some(id.clone());
        Ok(id)
    }

    /// End the current session with a summary.
    pub fn end_session(&mut self, summary: &str) -> Result<()> {
        let session_id = self
            .current_session_id
            .as_ref()
            .context("No active session to end")?;
        let now = now_epoch();

        self.conn.execute(
            "UPDATE sessions SET ended_at = ?1, summary = ?2 WHERE id = ?3",
            rusqlite::params![now, summary, session_id],
        )?;

        self.current_session_id = None;
        Ok(())
    }

    /// Save a message to the current session.
    ///
    /// SECURITY (CAT 3 / CAT 9): Both `content` and the JSON-serialized
    /// `tool_calls` are passed through `permissions::sensitive_filter::redact_sensitive`
    /// before being written to SQLite. Pasted credentials (API keys, bearer
    /// tokens, AWS access keys, GitHub PATs, private keys, etc.) are
    /// replaced with `[REDACTED]` markers so they are not persisted in
    /// `~/.ftai/sessions.db` and re-injected into future system prompts.
    /// AUDIT P0 #9.
    pub fn save_message(
        &self,
        role: Role,
        content: &str,
        tool_calls: Option<&[ToolCall]>,
    ) -> Result<()> {
        let session_id = self
            .current_session_id
            .as_ref()
            .context("No active session")?;

        let seq = self.next_seq(session_id)?;
        let now = now_epoch();
        let role_str = role_to_str(&role);

        let safe_content = crate::permissions::sensitive_filter::redact_sensitive(content);
        let tc_json = tool_calls.map(|tcs| {
            let raw = serde_json::to_string(tcs).unwrap_or_default();
            crate::permissions::sensitive_filter::redact_sensitive(&raw)
        });
        let tokens_est = crate::session::budget::TokenBudget::estimate_tokens(&safe_content);

        self.conn.execute(
            "INSERT INTO messages (session_id, seq, role, content, tool_calls, tokens_estimated, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![session_id, seq, role_str, safe_content, tc_json, tokens_est, now],
        )?;

        // Update message count.
        self.conn.execute(
            "UPDATE sessions SET message_count = message_count + 1 WHERE id = ?1",
            [session_id],
        )?;

        Ok(())
    }

    /// Load all messages for the current session, in order.
    pub fn load_current_messages(&self) -> Result<Vec<Message>> {
        let session_id = self
            .current_session_id
            .as_ref()
            .context("No active session")?;
        self.load_messages_for_session(session_id)
    }

    /// Load the most recent completed session's summary for this project,
    /// but only if it ended less than 24 hours ago.
    pub fn load_previous_summary(&self) -> Result<Option<String>> {
        let now = now_epoch();
        let cutoff = now - 24 * 60 * 60; // 24 hours ago

        let result: Option<String> = self
            .conn
            .query_row(
                "SELECT summary FROM sessions
                 WHERE project = ?1 AND ended_at IS NOT NULL AND ended_at > ?2
                 ORDER BY ended_at DESC LIMIT 1",
                rusqlite::params![self.project, cutoff],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        Ok(result)
    }

    /// Get the current session ID, if any.
    pub fn current_session_id(&self) -> Option<&str> {
        self.current_session_id.as_deref()
    }

    /// Resume a specific session by ID. Loads its messages.
    pub fn resume_session(&mut self, session_id: &str) -> Result<Vec<Message>> {
        // Verify the session exists and belongs to this project
        let exists: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sessions WHERE id = ?1 AND project = ?2",
            rusqlite::params![session_id, self.project],
            |r| r.get(0),
        )?;

        if !exists {
            anyhow::bail!("Session '{session_id}' not found for this project");
        }

        self.current_session_id = Some(session_id.to_string());
        self.load_messages_for_session(session_id)
    }

    /// Resume the most recent session for this project.
    pub fn resume_latest(&mut self) -> Result<Option<(String, Vec<Message>)>> {
        let latest: Option<String> = self.conn.query_row(
            "SELECT id FROM sessions WHERE project = ?1 ORDER BY started_at DESC LIMIT 1",
            [&self.project],
            |r| r.get(0),
        ).ok();

        match latest {
            Some(id) => {
                let messages = self.resume_session(&id)?;
                Ok(Some((id, messages)))
            }
            None => Ok(None),
        }
    }

    /// List recent sessions for this project.
    pub fn list_recent(&self, limit: usize) -> Result<Vec<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, ended_at, summary, message_count
             FROM sessions WHERE project = ?1
             ORDER BY started_at DESC LIMIT ?2",
        )?;

        let sessions = stmt
            .query_map(rusqlite::params![self.project, limit as i64], |row| {
                Ok(SessionSummary {
                    id: row.get(0)?,
                    started_at: row.get(1)?,
                    ended_at: row.get(2)?,
                    summary: row.get(3)?,
                    message_count: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(sessions)
    }

    fn next_seq(&self, session_id: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            [session_id],
            |r| r.get(0),
        )?;
        Ok(count)
    }

    fn load_messages_for_session(&self, session_id: &str) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content, tool_calls, tool_call_id
             FROM messages WHERE session_id = ?1 ORDER BY seq",
        )?;

        let messages = stmt
            .query_map([session_id], |row| {
                let role_str: String = row.get(0)?;
                let content: String = row.get(1)?;
                let tc_json: Option<String> = row.get(2)?;
                let tool_call_id: Option<String> = row.get(3)?;

                let tool_calls = tc_json.and_then(|json| serde_json::from_str(&json).ok());

                Ok(Message {
                    role: str_to_role(&role_str),
                    content,
                    tool_calls,
                    tool_call_id,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(messages)
    }
}

fn role_to_str(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn str_to_role(s: &str) -> Role {
    match s {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn test_manager(project: &str) -> SessionManager {
        let tmp = NamedTempFile::new().unwrap();
        SessionManager::open(tmp.path(), project).unwrap()
    }

    #[test]
    fn test_session_lifecycle() {
        let mut mgr = test_manager("test-proj");

        let id = mgr.start_session().unwrap();
        assert!(!id.is_empty());
        assert!(mgr.current_session_id().is_some());

        mgr.save_message(Role::User, "Hello", None).unwrap();
        mgr.save_message(Role::Assistant, "Hi there", None).unwrap();

        let messages = mgr.load_current_messages().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content, "Hello");
        assert_eq!(messages[1].role, Role::Assistant);

        mgr.end_session("Greeted the user").unwrap();
        assert!(mgr.current_session_id().is_none());
    }

    #[test]
    fn test_save_message_with_tool_calls() {
        let mut mgr = test_manager("proj");
        mgr.start_session().unwrap();

        let tool_calls = vec![ToolCall {
            id: "tc1".to_string(),
            name: "file_read".to_string(),
            arguments: serde_json::json!({"path": "/src/main.rs"}),
        }];

        mgr.save_message(Role::Assistant, "Let me read that", Some(&tool_calls))
            .unwrap();

        let messages = mgr.load_current_messages().unwrap();
        assert_eq!(messages.len(), 1);
        let tcs = messages[0].tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].name, "file_read");
    }

    #[test]
    fn test_no_session_errors() {
        let mgr = test_manager("proj");
        // No session started — operations should fail.
        assert!(mgr.save_message(Role::User, "hi", None).is_err());
        assert!(mgr.load_current_messages().is_err());
    }

    #[test]
    fn test_previous_summary_within_24h() {
        let tmp = NamedTempFile::new().unwrap();
        let mut mgr = SessionManager::open(tmp.path(), "proj").unwrap();

        // Create and end a session.
        mgr.start_session().unwrap();
        mgr.save_message(Role::User, "Do stuff", None).unwrap();
        mgr.end_session("Did some stuff").unwrap();

        // New manager, same DB and project.
        let mgr2 = SessionManager::open(tmp.path(), "proj").unwrap();
        let summary = mgr2.load_previous_summary().unwrap();
        assert_eq!(summary.as_deref(), Some("Did some stuff"));
    }

    #[test]
    fn test_previous_summary_different_project() {
        let tmp = NamedTempFile::new().unwrap();
        let mut mgr = SessionManager::open(tmp.path(), "proj-a").unwrap();
        mgr.start_session().unwrap();
        mgr.end_session("Summary for A").unwrap();

        let mgr2 = SessionManager::open(tmp.path(), "proj-b").unwrap();
        let summary = mgr2.load_previous_summary().unwrap();
        assert!(summary.is_none(), "Should not see project A's summary");
    }

    #[test]
    fn test_previous_summary_expired() {
        let tmp = NamedTempFile::new().unwrap();

        // Manually insert an old session (ended 25h ago).
        let conn = rusqlite::Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY, project TEXT NOT NULL, started_at INTEGER NOT NULL,
                ended_at INTEGER, summary TEXT, message_count INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0
            )",
        )
        .unwrap();

        let old_time = now_epoch() - 25 * 60 * 60;
        conn.execute(
            "INSERT INTO sessions (id, project, started_at, ended_at, summary)
             VALUES ('old', 'proj', ?1, ?2, 'old summary')",
            rusqlite::params![old_time - 100, old_time],
        )
        .unwrap();
        drop(conn);

        let mgr = SessionManager::open(tmp.path(), "proj").unwrap();
        let summary = mgr.load_previous_summary().unwrap();
        assert!(summary.is_none(), "Summary older than 24h should not load");
    }

    #[test]
    fn test_message_ordering() {
        let mut mgr = test_manager("proj");
        mgr.start_session().unwrap();

        for i in 0..10 {
            mgr.save_message(Role::User, &format!("msg-{i}"), None)
                .unwrap();
        }

        let messages = mgr.load_current_messages().unwrap();
        assert_eq!(messages.len(), 10);
        for (i, msg) in messages.iter().enumerate() {
            assert_eq!(msg.content, format!("msg-{i}"));
        }
    }
}
