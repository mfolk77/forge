// FTAI Session & Evolution Modules -- Security Red Tests
// FolkTech Secure Coding Standard
//
// Tests cover:
// - SQL injection via message content, summaries, tool call JSON (session/manager.rs)
// - Session ID unpredictability (session/manager.rs)
// - Oversized payload handling (session/manager.rs)
// - SQL injection via task descriptions, error messages, tool names (evolution/store.rs)
// - P0: FTAI DSL injection via tool names in generated rules (evolution/generator.rs)
// - P0: FTAI DSL injection via error messages in generated rules (evolution/generator.rs)
// - P1: FTAI DSL injection via project names in generated rules (evolution/generator.rs)

#[cfg(test)]
mod security_tests {
    // ===================================================================
    // Session Manager Security Tests
    // ===================================================================
    mod session_manager {
        use crate::backend::types::{Role, ToolCall};
        use crate::session::manager::SessionManager;
        use tempfile::NamedTempFile;

        fn test_manager(project: &str) -> SessionManager {
            let tmp = NamedTempFile::new().unwrap();
            SessionManager::open(tmp.path(), project).unwrap()
        }

        // ---------------------------------------------------------------
        // P0: SQL injection via message content
        // ---------------------------------------------------------------

        #[test]
        fn test_p0_session_sql_injection_via_message_content() {
            // ATTACK: User sends a message containing SQL injection payload.
            //         This is extremely common -- users paste SQL code, error
            //         messages with SQL, etc.
            // EXPECT: Parameterized queries neutralize the injection.
            // VERIFY: Message is stored and retrieved literally.
            let mut mgr = test_manager("test-proj");
            mgr.start_session().unwrap();

            let injection = "'; DROP TABLE messages; --";
            mgr.save_message(Role::User, injection, None).unwrap();

            let messages = mgr.load_current_messages().unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].content, injection);
        }

        #[test]
        fn test_p0_session_sql_injection_via_content_union_select() {
            // ATTACK: UNION SELECT injection in message content.
            let mut mgr = test_manager("test-proj");
            mgr.start_session().unwrap();

            let injection = "' UNION SELECT id, session_id, 'x', 'x', 'x', 'x', 0, 0 FROM messages --";
            mgr.save_message(Role::User, injection, None).unwrap();
            mgr.save_message(Role::Assistant, "response", None).unwrap();

            let messages = mgr.load_current_messages().unwrap();
            assert_eq!(messages.len(), 2, "Only 2 real messages should exist, not injected rows");
            assert_eq!(messages[0].content, injection);
        }

        #[test]
        fn test_p0_session_sql_injection_via_summary() {
            // ATTACK: Session summary contains SQL injection.
            let tmp = NamedTempFile::new().unwrap();
            let mut mgr = SessionManager::open(tmp.path(), "proj").unwrap();
            mgr.start_session().unwrap();

            let malicious_summary = "Done'; DROP TABLE sessions; --";
            mgr.end_session(malicious_summary).unwrap();

            // Reopen and verify the summary is stored literally.
            let mgr2 = SessionManager::open(tmp.path(), "proj").unwrap();
            let summary = mgr2.load_previous_summary().unwrap();
            assert_eq!(summary.as_deref(), Some(malicious_summary));
        }

        #[test]
        fn test_p0_session_sql_injection_via_project_name() {
            // ATTACK: Project name contains SQL injection.
            let tmp = NamedTempFile::new().unwrap();
            let malicious_project = "'; DELETE FROM sessions; --";
            let mut mgr = SessionManager::open(tmp.path(), malicious_project).unwrap();

            mgr.start_session().unwrap();
            mgr.save_message(Role::User, "test", None).unwrap();
            mgr.end_session("summary").unwrap();

            // Reopen with the same malicious project name.
            let mgr2 = SessionManager::open(tmp.path(), malicious_project).unwrap();
            let summary = mgr2.load_previous_summary().unwrap();
            assert_eq!(summary.as_deref(), Some("summary"));
        }

        // ---------------------------------------------------------------
        // P0: SQL injection via tool call JSON
        // ---------------------------------------------------------------

        #[test]
        fn test_p0_session_sql_injection_via_tool_call_json() {
            // ATTACK: Tool call contains SQL injection in its arguments.
            let mut mgr = test_manager("proj");
            mgr.start_session().unwrap();

            let tool_calls = vec![ToolCall {
                id: "tc-evil".to_string(),
                name: "bash".to_string(),
                arguments: serde_json::json!({
                    "command": "'; DROP TABLE messages; --"
                }),
            }];

            mgr.save_message(Role::Assistant, "running command", Some(&tool_calls))
                .unwrap();

            let messages = mgr.load_current_messages().unwrap();
            assert_eq!(messages.len(), 1);
            let tcs = messages[0].tool_calls.as_ref().unwrap();
            assert_eq!(tcs[0].arguments["command"], "'; DROP TABLE messages; --");
        }

        #[test]
        fn test_p0_session_sql_injection_via_tool_call_name() {
            // ATTACK: Tool name itself is a SQL injection payload.
            let mut mgr = test_manager("proj");
            mgr.start_session().unwrap();

            let tool_calls = vec![ToolCall {
                id: "tc1".to_string(),
                name: "'; DROP TABLE messages; --".to_string(),
                arguments: serde_json::json!({}),
            }];

            mgr.save_message(Role::Assistant, "call", Some(&tool_calls))
                .unwrap();

            let messages = mgr.load_current_messages().unwrap();
            let tcs = messages[0].tool_calls.as_ref().unwrap();
            // Tool name is inside JSON, which is stored as a TEXT blob.
            // The JSON serialization handles the escaping.
            assert_eq!(tcs[0].name, "'; DROP TABLE messages; --");
        }

        // ---------------------------------------------------------------
        // P1: Session ID unpredictability
        // ---------------------------------------------------------------

        #[test]
        fn test_p1_session_id_is_uuid_v4_unpredictable() {
            // ATTACK: If session IDs are predictable, an attacker could guess
            //         another user's session ID and load their messages.
            // VERIFY: Session IDs are UUID v4 (random), and no two are alike.
            let mut mgr = test_manager("proj");
            let id1 = mgr.start_session().unwrap();
            mgr.end_session("done").unwrap();
            let id2 = mgr.start_session().unwrap();

            assert_ne!(id1, id2, "Session IDs must be unique");
            // UUID v4 format: 8-4-4-4-12 hex chars.
            assert_eq!(id1.len(), 36);
            assert_eq!(id2.len(), 36);
            assert_eq!(id1.chars().filter(|c| *c == '-').count(), 4);
            // Version nibble (position 14) should be '4' for UUID v4.
            assert_eq!(
                id1.chars().nth(14),
                Some('4'),
                "Session ID should be UUID v4"
            );
        }

        // ---------------------------------------------------------------
        // P2: Oversized payload handling
        // ---------------------------------------------------------------

        #[test]
        fn test_p2_session_oversized_message_content() {
            // ATTACK: A 10MB message is saved to the session database.
            // EXPECT: SQLite handles it without crashing. No explicit size
            //         limit is enforced -- this test documents the gap.
            // VERIFY: The message is stored and can be retrieved.
            let mut mgr = test_manager("proj");
            mgr.start_session().unwrap();

            let large_content = "x".repeat(10 * 1024 * 1024); // 10 MB
            mgr.save_message(Role::User, &large_content, None).unwrap();

            let messages = mgr.load_current_messages().unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].content.len(), 10 * 1024 * 1024);
            // NOTE: This succeeds, confirming no size limit. A size limit
            // should be added to prevent DB bloat / resource exhaustion.
        }

        #[test]
        fn test_p2_session_oversized_tool_calls_json() {
            // ATTACK: Tool call with extremely large arguments JSON.
            let mut mgr = test_manager("proj");
            mgr.start_session().unwrap();

            let large_args = serde_json::json!({
                "data": "x".repeat(1_000_000)
            });
            let tool_calls = vec![ToolCall {
                id: "tc-big".to_string(),
                name: "file_write".to_string(),
                arguments: large_args,
            }];

            mgr.save_message(Role::Assistant, "writing", Some(&tool_calls))
                .unwrap();

            let messages = mgr.load_current_messages().unwrap();
            assert_eq!(messages.len(), 1);
        }

        // ---------------------------------------------------------------
        // P2: Message content with control characters
        // ---------------------------------------------------------------

        #[test]
        fn test_p2_session_message_with_null_bytes() {
            // ATTACK: Message content contains null bytes.
            // VERIFY: SQLite TEXT columns handle null bytes (they shouldn't
            //         appear in TEXT, but rusqlite may handle it).
            let mut mgr = test_manager("proj");
            mgr.start_session().unwrap();

            let content_with_nulls = "hello\0world\0";
            mgr.save_message(Role::User, content_with_nulls, None)
                .unwrap();

            let messages = mgr.load_current_messages().unwrap();
            assert_eq!(messages.len(), 1);
            // SQLite TEXT may truncate at null or store the whole thing
            // depending on the driver. Either behavior is acceptable.
        }

        #[test]
        fn test_p2_session_message_with_unicode_exploits() {
            // ATTACK: Message with RTL override, zero-width chars, homoglyphs.
            let mut mgr = test_manager("proj");
            mgr.start_session().unwrap();

            let tricky = "normal \u{200B}zero-width \u{202E}RTL-override \u{FEFF}BOM";
            mgr.save_message(Role::User, tricky, None).unwrap();

            let messages = mgr.load_current_messages().unwrap();
            assert_eq!(messages[0].content, tricky);
        }
    }

    // ===================================================================
    // Evolution Store Security Tests
    // ===================================================================
    mod evolution_store {
        use crate::evolution::analyzer::*;
        use crate::evolution::generator::*;
        use crate::evolution::store::EvolutionStore;
        use tempfile::NamedTempFile;

        fn test_store() -> EvolutionStore {
            let tmp = NamedTempFile::new().unwrap();
            EvolutionStore::open(tmp.path()).unwrap()
        }

        fn sample_outcome(
            id: &str,
            project: &str,
            task: &str,
            outcome: OutcomeType,
            tools: Vec<(&str, ToolResultType)>,
        ) -> SessionOutcome {
            SessionOutcome {
                session_id: id.to_string(),
                project: project.to_string(),
                timestamp: 1711500000,
                task_description: task.to_string(),
                tool_calls: tools
                    .into_iter()
                    .enumerate()
                    .map(|(seq, (name, result))| ToolCallRecord {
                        tool_name: name.to_string(),
                        arguments_summary: format!("arg-{seq}"),
                        result_type: result,
                        duration_ms: 10,
                    })
                    .collect(),
                success: outcome,
                user_feedback: None,
                total_tokens: 1000,
                retries: 0,
            }
        }

        // ---------------------------------------------------------------
        // P0: SQL injection via task description
        // ---------------------------------------------------------------

        #[test]
        fn test_p0_evolution_sql_injection_via_task_description() {
            // ATTACK: Task description contains SQL injection payload.
            let store = test_store();
            let malicious_task = "'; DROP TABLE sessions; --";
            let outcome = sample_outcome(
                "s1",
                "proj",
                malicious_task,
                OutcomeType::Success,
                vec![("file_read", ToolResultType::Success)],
            );
            store.save_outcome(&outcome).unwrap();

            let loaded = store.recent_sessions(10).unwrap();
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded[0].task_description, malicious_task);
        }

        // ---------------------------------------------------------------
        // P0: SQL injection via error messages in tool calls
        // ---------------------------------------------------------------

        #[test]
        fn test_p0_evolution_sql_injection_via_error_message() {
            // ATTACK: Error message from a tool contains SQL injection.
            let store = test_store();
            let malicious_error = "error: '; DROP TABLE tool_calls; --";
            let outcome = sample_outcome(
                "s1",
                "proj",
                "task",
                OutcomeType::Failure("err".into()),
                vec![("bash", ToolResultType::Error(malicious_error.to_string()))],
            );
            store.save_outcome(&outcome).unwrap();

            let loaded = store.recent_sessions(10).unwrap();
            assert_eq!(loaded[0].tool_calls.len(), 1);
            assert_eq!(
                loaded[0].tool_calls[0].result_type,
                ToolResultType::Error(malicious_error.to_string())
            );
        }

        // ---------------------------------------------------------------
        // P0: SQL injection via tool name
        // ---------------------------------------------------------------

        #[test]
        fn test_p0_evolution_sql_injection_via_tool_name() {
            // ATTACK: Tool name contains SQL injection.
            let store = test_store();
            let malicious_tool = "'; DELETE FROM tool_calls; --";
            let outcome = sample_outcome(
                "s1",
                "proj",
                "task",
                OutcomeType::Success,
                vec![(malicious_tool, ToolResultType::Success)],
            );
            store.save_outcome(&outcome).unwrap();

            let loaded = store.recent_sessions(10).unwrap();
            assert_eq!(loaded[0].tool_calls[0].tool_name, malicious_tool);
        }

        // ---------------------------------------------------------------
        // P0: SQL injection via generated rule name and content
        // ---------------------------------------------------------------

        #[test]
        fn test_p0_evolution_sql_injection_via_rule_name() {
            // ATTACK: Rule name contains SQL injection.
            let store = test_store();
            let rule = GeneratedRule {
                name: "'; DROP TABLE generated_rules; --".to_string(),
                source: RuleSource::OrderingPattern,
                confidence: 0.9,
                ftai_rule: "rule x { }".to_string(),
            };
            store.save_generated_rule(&rule).unwrap();

            let active = store.active_rules().unwrap();
            assert_eq!(active.len(), 1);
            assert_eq!(active[0].name, "'; DROP TABLE generated_rules; --");
        }

        #[test]
        fn test_p0_evolution_sql_injection_via_rule_content() {
            // ATTACK: Rule FTAI DSL text contains SQL injection.
            let store = test_store();
            let rule = GeneratedRule {
                name: "test-rule".to_string(),
                source: RuleSource::RepeatedFailure,
                confidence: 0.85,
                ftai_rule: "rule x { '; DROP TABLE generated_rules; -- }".to_string(),
            };
            store.save_generated_rule(&rule).unwrap();

            let active = store.active_rules().unwrap();
            assert_eq!(active[0].ftai_rule, "rule x { '; DROP TABLE generated_rules; -- }");
        }
    }

    // ===================================================================
    // Evolution Generator Security Tests -- DSL INJECTION
    // ===================================================================
    mod evolution_generator {
        use crate::evolution::analyzer::*;
        use crate::evolution::generator::*;
        use crate::evolution::store::EvolutionStore;
        use tempfile::NamedTempFile;

        fn make_store() -> EvolutionStore {
            let tmp = NamedTempFile::new().unwrap();
            EvolutionStore::open(tmp.path()).unwrap()
        }

        fn make_outcome(
            id: &str,
            project: &str,
            outcome: OutcomeType,
            tools: Vec<(&str, ToolResultType)>,
        ) -> SessionOutcome {
            SessionOutcome {
                session_id: id.to_string(),
                project: project.to_string(),
                timestamp: 1711500000,
                task_description: "task".to_string(),
                tool_calls: tools
                    .into_iter()
                    .map(|(name, result)| ToolCallRecord {
                        tool_name: name.to_string(),
                        arguments_summary: String::new(),
                        result_type: result,
                        duration_ms: 10,
                    })
                    .collect(),
                success: outcome,
                user_feedback: None,
                total_tokens: 1000,
                retries: 0,
            }
        }

        // ---------------------------------------------------------------
        // P0: FTAI DSL injection via tool names in ordering patterns
        // ---------------------------------------------------------------

        #[test]
        fn test_p0_generator_dsl_injection_via_tool_name_in_ordering() {
            // ATTACK: A tool name contains FTAI DSL metacharacters.
            //         If tool_name = 'file_read")\n  reject "pwned', the
            //         generated rule would contain injected DSL directives:
            //
            //         rule evo-order-file_read")\n  reject "pwned-before-file_edit {
            //           scope "*"
            //           on tool_call
            //           when sequence("file_read")
            //             reject "pwned", "file_edit")
            //           ...
            //         }
            //
            //         This corrupts the rule and could inject a reject directive.
            // EXPECT: Tool names should be validated/sanitized before rule generation.
            // VERIFY: The generated rule either rejects the malicious name or
            //         sanitizes it. Currently it does NOT -- this test proves the vuln.
            let store = make_store();
            let engine = EvolutionEngine::with_min_sessions(store, 1);

            // The malicious tool name that would inject DSL.
            let evil_tool = "file_read\")\n  reject \"all_tools_blocked";

            for i in 0..7 {
                let outcome = make_outcome(
                    &format!("s{i}"),
                    "proj",
                    OutcomeType::Success,
                    vec![
                        (evil_tool, ToolResultType::Success),
                        ("file_edit", ToolResultType::Success),
                    ],
                );
                engine.analyze_and_evolve(&outcome).unwrap();
            }

            let outcome = make_outcome(
                "final",
                "proj",
                OutcomeType::Success,
                vec![
                    (evil_tool, ToolResultType::Success),
                    ("file_edit", ToolResultType::Success),
                ],
            );
            let rules = engine.analyze_and_evolve(&outcome).unwrap();

            // Find the ordering rule that was generated.
            let ordering_rules: Vec<&GeneratedRule> = rules
                .iter()
                .filter(|r| r.source == RuleSource::OrderingPattern)
                .collect();

            for rule in &ordering_rules {
                // VULNERABILITY CHECK: The generated rule text should NOT
                // contain unescaped newlines or injected DSL directives.
                // If it does, the tool name was interpolated without sanitization.
                let has_injected_reject = rule.ftai_rule.contains("reject \"all_tools_blocked");
                let has_raw_newline_in_name = rule.name.contains('\n');

                // CURRENT STATE: This assertion documents the vulnerability.
                // When the fix is applied, flip this to assert the OPPOSITE.
                if has_injected_reject || has_raw_newline_in_name {
                    panic!(
                        "P0 VULNERABILITY CONFIRMED: FTAI DSL injection via tool name.\n\
                         Generated rule contains injected reject directive.\n\
                         Rule name: {:?}\n\
                         Rule body:\n{}\n\
                         FIX REQUIRED: Sanitize tool names before interpolating into DSL.\n\
                         Strip or reject names containing: quotes, braces, newlines, backslashes.",
                        rule.name, rule.ftai_rule
                    );
                }
            }
            // If no ordering rules were generated (e.g., the malicious name
            // prevented the pattern from being detected), that's also acceptable
            // as a form of implicit mitigation.
        }

        // ---------------------------------------------------------------
        // P0: FTAI DSL injection via error messages in failure patterns
        // ---------------------------------------------------------------

        #[test]
        fn test_p0_generator_dsl_injection_via_error_message() {
            // ATTACK: Error message from a tool contains DSL injection.
            //         The generator's detect_repeated_failures only escapes
            //         double quotes with replace('"', '\\"') but does NOT
            //         escape newlines, braces, or other DSL metacharacters.
            //
            //         Error: 'permission denied\n  reject "all"\n}'
            //         Would produce:
            //         rule evo-fail-bash-abc123 {
            //           ...
            //           reject "Repeated failure pattern: permission denied
            //             reject "all"
            //           }
            //           ...
            //         }
            let store = make_store();
            let engine = EvolutionEngine::with_min_sessions(store, 1);

            // Error message with DSL injection payload.
            let evil_error = "permission denied\n  reject \"block_everything\"\n}";

            for i in 0..7 {
                let outcome = make_outcome(
                    &format!("fail-{i}"),
                    "proj",
                    OutcomeType::Failure("err".into()),
                    vec![("bash", ToolResultType::Error(evil_error.to_string()))],
                );
                engine.analyze_and_evolve(&outcome).unwrap();
            }

            let outcome = make_outcome(
                "fail-final",
                "proj",
                OutcomeType::Failure("err".into()),
                vec![("bash", ToolResultType::Error(evil_error.to_string()))],
            );
            let rules = engine.analyze_and_evolve(&outcome).unwrap();

            let failure_rules: Vec<&GeneratedRule> = rules
                .iter()
                .filter(|r| r.source == RuleSource::RepeatedFailure)
                .collect();

            for rule in &failure_rules {
                // Check if the injected reject directive made it into the rule.
                let lines: Vec<&str> = rule.ftai_rule.lines().collect();
                let reject_count = lines
                    .iter()
                    .filter(|l| l.trim().starts_with("reject"))
                    .count();

                if reject_count > 1 {
                    panic!(
                        "P0 VULNERABILITY CONFIRMED: FTAI DSL injection via error message.\n\
                         Generated rule has {} reject directives (expected 1).\n\
                         Rule body:\n{}\n\
                         FIX REQUIRED: Escape or strip newlines and DSL metacharacters \
                         from error messages before interpolating into rule text.\n\
                         The current replace('\"', '\\\"') is insufficient.",
                        reject_count, rule.ftai_rule
                    );
                }
            }
        }

        // ---------------------------------------------------------------
        // P1: FTAI DSL injection via project names
        // ---------------------------------------------------------------

        #[test]
        fn test_p1_generator_dsl_injection_via_project_name() {
            // ATTACK: Project name contains DSL metacharacters.
            //         detect_project_patterns interpolates the project name
            //         into both the rule name and scope directive.
            //
            //         Project: '*" }\nrule evil { scope "*" reject "all"'
            //         Would produce a rule with an injected second rule block.
            let store = make_store();
            let engine = EvolutionEngine::with_min_sessions(store, 1);

            let evil_project = "proj\"\n}\nrule evil {\n  scope \"*\"\n  reject \"all\"";

            // Need MIN_OBSERVATIONS (5) sessions for the project pattern.
            for i in 0..7 {
                let outcome = SessionOutcome {
                    session_id: format!("proj-{i}"),
                    project: evil_project.to_string(),
                    timestamp: 1711500000,
                    task_description: "task".to_string(),
                    tool_calls: vec![ToolCallRecord {
                        tool_name: "file_read".to_string(),
                        arguments_summary: String::new(),
                        result_type: ToolResultType::Success,
                        duration_ms: 10,
                    }],
                    success: OutcomeType::Success,
                    user_feedback: None,
                    total_tokens: 1000,
                    retries: 0,
                };
                engine.analyze_and_evolve(&outcome).unwrap();
            }

            let outcome = SessionOutcome {
                session_id: "proj-final".to_string(),
                project: evil_project.to_string(),
                timestamp: 1711500000,
                task_description: "task".to_string(),
                tool_calls: vec![ToolCallRecord {
                    tool_name: "file_read".to_string(),
                    arguments_summary: String::new(),
                    result_type: ToolResultType::Success,
                    duration_ms: 10,
                }],
                success: OutcomeType::Success,
                user_feedback: None,
                total_tokens: 1000,
                retries: 0,
            };
            let rules = engine.analyze_and_evolve(&outcome).unwrap();

            let project_rules: Vec<&GeneratedRule> = rules
                .iter()
                .filter(|r| r.source == RuleSource::ProjectPattern)
                .collect();

            for rule in &project_rules {
                // Count how many top-level "rule " declarations exist.
                let rule_decl_count = rule
                    .ftai_rule
                    .lines()
                    .filter(|l| l.trim().starts_with("rule "))
                    .count();

                if rule_decl_count > 1 {
                    panic!(
                        "P1 VULNERABILITY CONFIRMED: DSL injection via project name.\n\
                         Generated rule contains {} rule declarations (expected 1).\n\
                         Rule body:\n{}\n\
                         FIX REQUIRED: Sanitize project names before DSL interpolation.",
                        rule_decl_count, rule.ftai_rule
                    );
                }
            }
        }

        // ---------------------------------------------------------------
        // P0: Tool name sanitization - what SHOULD be valid
        // ---------------------------------------------------------------

        #[test]
        fn test_p0_generator_safe_tool_names_produce_valid_rules() {
            // VERIFY: Normal tool names produce well-formed rules.
            let store = make_store();
            let engine = EvolutionEngine::with_min_sessions(store, 1);

            for i in 0..7 {
                let outcome = make_outcome(
                    &format!("s{i}"),
                    "proj",
                    OutcomeType::Success,
                    vec![
                        ("file_read", ToolResultType::Success),
                        ("file_edit", ToolResultType::Success),
                    ],
                );
                engine.analyze_and_evolve(&outcome).unwrap();
            }

            let outcome = make_outcome(
                "final",
                "proj",
                OutcomeType::Success,
                vec![
                    ("file_read", ToolResultType::Success),
                    ("file_edit", ToolResultType::Success),
                ],
            );
            let rules = engine.analyze_and_evolve(&outcome).unwrap();

            for rule in &rules {
                // Every generated rule should be well-formed DSL.
                assert!(
                    rule.ftai_rule.starts_with("rule "),
                    "Rule should start with 'rule' keyword: {}",
                    rule.ftai_rule
                );
                // Rule name should be alphanumeric with hyphens.
                assert!(
                    rule.name
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '-' || c == '_'),
                    "Rule name should be safe identifier: {:?}",
                    rule.name
                );
                // Brace count should be balanced.
                let open = rule.ftai_rule.chars().filter(|c| *c == '{').count();
                let close = rule.ftai_rule.chars().filter(|c| *c == '}').count();
                assert_eq!(
                    open, close,
                    "Rule braces should be balanced. Open: {}, Close: {}. Rule:\n{}",
                    open, close, rule.ftai_rule
                );
            }
        }

        // ---------------------------------------------------------------
        // P0: Error message escape validation
        // ---------------------------------------------------------------

        #[test]
        fn test_p0_generator_error_message_with_quotes_escaped() {
            // VERIFY: Double quotes in error messages are escaped in the rule body.
            let store = make_store();
            let engine = EvolutionEngine::with_min_sessions(store, 1);

            let error_with_quotes = "file \"not found\" in /path";

            for i in 0..7 {
                let outcome = make_outcome(
                    &format!("fail-{i}"),
                    "proj",
                    OutcomeType::Failure("err".into()),
                    vec![(
                        "file_read",
                        ToolResultType::Error(error_with_quotes.to_string()),
                    )],
                );
                engine.analyze_and_evolve(&outcome).unwrap();
            }

            let outcome = make_outcome(
                "fail-final",
                "proj",
                OutcomeType::Failure("err".into()),
                vec![(
                    "file_read",
                    ToolResultType::Error(error_with_quotes.to_string()),
                )],
            );
            let rules = engine.analyze_and_evolve(&outcome).unwrap();

            let failure_rules: Vec<&GeneratedRule> = rules
                .iter()
                .filter(|r| r.source == RuleSource::RepeatedFailure)
                .collect();

            for rule in &failure_rules {
                // The escaped version should appear, not the raw quotes.
                assert!(
                    rule.ftai_rule.contains("\\\"not found\\\""),
                    "Double quotes in error messages should be escaped in DSL.\nRule:\n{}",
                    rule.ftai_rule
                );
            }
        }

        // ---------------------------------------------------------------
        // P0: Error message with backslash sequences
        // ---------------------------------------------------------------

        #[test]
        fn test_p0_generator_error_message_with_backslashes() {
            // ATTACK: Error message contains backslashes that could escape
            //         the escape: \" becomes \\" which re-opens the quote.
            // VERIFY: Backslashes should also be escaped before quotes.
            let store = make_store();
            let engine = EvolutionEngine::with_min_sessions(store, 1);

            // Payload: the backslash before the quote would cancel the
            // existing quote-escape if backslashes aren't handled.
            let evil_error = r#"path\"; reject "pwned"#;

            for i in 0..7 {
                let outcome = make_outcome(
                    &format!("bs-{i}"),
                    "proj",
                    OutcomeType::Failure("err".into()),
                    vec![("bash", ToolResultType::Error(evil_error.to_string()))],
                );
                engine.analyze_and_evolve(&outcome).unwrap();
            }

            let outcome = make_outcome(
                "bs-final",
                "proj",
                OutcomeType::Failure("err".into()),
                vec![("bash", ToolResultType::Error(evil_error.to_string()))],
            );
            let rules = engine.analyze_and_evolve(&outcome).unwrap();

            let failure_rules: Vec<&GeneratedRule> = rules
                .iter()
                .filter(|r| r.source == RuleSource::RepeatedFailure)
                .collect();

            for rule in &failure_rules {
                // The injected reject should NOT appear as a separate directive.
                let lines: Vec<&str> = rule.ftai_rule.lines().collect();
                let reject_lines: Vec<&&str> = lines
                    .iter()
                    .filter(|l| l.trim().starts_with("reject"))
                    .collect();

                if reject_lines.len() > 1 {
                    panic!(
                        "P0 VULNERABILITY CONFIRMED: Backslash escape bypass in error message.\n\
                         Rule has {} reject directives (expected 1).\n\
                         Rule body:\n{}\n\
                         FIX REQUIRED: Escape backslashes BEFORE escaping quotes.",
                        reject_lines.len(),
                        rule.ftai_rule
                    );
                }
            }
        }

        // ---------------------------------------------------------------
        // P1: Generated rule text loaded without validation
        // ---------------------------------------------------------------

        #[test]
        fn test_p1_evolution_store_loads_arbitrary_rule_text() {
            // ATTACK: If an attacker can write to the evolution DB, they can
            //         insert arbitrary rule text that gets loaded by active_rules.
            // VERIFY: active_rules returns whatever is in the DB with no validation.
            //         This test documents that the store does NO validation on load.
            let store = make_store();
            let malicious_rule = GeneratedRule {
                name: "evil-rule".to_string(),
                source: RuleSource::OrderingPattern,
                confidence: 0.99,
                ftai_rule: "rule evil {\n  scope \"*\"\n  reject \"*\"\n  # This blocks ALL tool calls\n}".to_string(),
            };
            store.save_generated_rule(&malicious_rule).unwrap();

            let active = store.active_rules().unwrap();
            assert_eq!(active.len(), 1);
            assert!(
                active[0].ftai_rule.contains("reject \"*\""),
                "Store loads arbitrary rule text without validation -- \
                 the rule evaluator must validate rules on load"
            );
        }
    }
}
