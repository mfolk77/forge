use std::collections::HashMap;
use std::path::Path;

use super::builtins;
use super::parser::{BinOperator, Event, Expression, Parser, Rule, RuleAction as ParsedAction, RuleSet};

/// Result of evaluating rules against an action
#[derive(Debug, Clone, PartialEq)]
pub enum RuleAction {
    Allow,
    Reject(String),  // reason
    Modify(String),  // modification description
}

/// Context for rule evaluation — what's happening right now
#[derive(Debug, Clone)]
pub struct EvalContext {
    pub event: Event,
    pub variables: HashMap<String, EvalValue>,
}

#[derive(Debug, Clone)]
pub enum EvalValue {
    String(String),
    Bool(bool),
    Number(f64),
    List(Vec<String>),
}

impl EvalContext {
    pub fn new(event: Event) -> Self {
        Self {
            event,
            variables: HashMap::new(),
        }
    }

    pub fn set(&mut self, key: &str, value: EvalValue) -> &mut Self {
        self.variables.insert(key.to_string(), value);
        self
    }

    pub fn set_str(&mut self, key: &str, value: &str) -> &mut Self {
        self.set(key, EvalValue::String(value.to_string()))
    }

    pub fn set_bool(&mut self, key: &str, value: bool) -> &mut Self {
        self.set(key, EvalValue::Bool(value))
    }
}

/// The rules engine — loads, caches, and evaluates rules
pub struct RulesEngine {
    global_rules: Vec<Rule>,
    scoped_rules: Vec<(String, Vec<Rule>)>, // (path_pattern, rules)
}

impl RulesEngine {
    pub fn new() -> Self {
        Self {
            global_rules: Vec::new(),
            scoped_rules: Vec::new(),
        }
    }

    /// Load rules from a .ftai DSL string
    pub fn load(&mut self, input: &str) -> Result<usize, String> {
        let ruleset = Parser::parse(input)?;
        let count = ruleset.rules.len() + ruleset.scopes.iter().map(|s| s.rules.len()).sum::<usize>();

        self.global_rules.extend(ruleset.rules);

        for scope in ruleset.scopes {
            self.scoped_rules.push((scope.path, scope.rules));
        }

        Ok(count)
    }

    /// Load rules from a file
    pub fn load_file(&mut self, path: &Path) -> Result<usize, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read rules file {}: {}", path.display(), e))?;
        self.load(&content)
    }

    /// Load rules from a string
    pub fn load_string(&mut self, content: &str) -> Result<usize, String> {
        self.load(content)
    }

    /// Load glob-matched rules from a directory.
    ///
    /// Scans `dir` for `.ftai` rule files with optional YAML frontmatter,
    /// filters by glob match against `file_path`, and loads matching rules
    /// via `load_string()`. Returns the total count of rules loaded.
    pub fn load_glob_rules(&mut self, dir: &Path, file_path: Option<&str>) -> Result<usize, String> {
        let glob_rules = super::glob_matcher::load_rules_for_context(dir, file_path);
        let mut total = 0;
        for gr in &glob_rules {
            if !gr.rule_content.trim().is_empty() {
                total += self.load_string(&gr.rule_content)?;
            }
        }
        Ok(total)
    }

    /// Clear all loaded rules
    pub fn clear(&mut self) {
        self.global_rules.clear();
        self.scoped_rules.clear();
    }

    /// Evaluate all matching rules for the given context
    pub fn evaluate(&self, ctx: &EvalContext, project_path: Option<&str>) -> RuleAction {
        let mut applicable_rules: Vec<&Rule> = Vec::new();

        // Global rules
        for rule in &self.global_rules {
            if event_matches(&rule.event, &ctx.event) {
                applicable_rules.push(rule);
            }
        }

        // Scoped rules
        if let Some(project) = project_path {
            for (scope_path, rules) in &self.scoped_rules {
                let expanded = scope_path.replace('~', &dirs::home_dir().unwrap_or_default().to_string_lossy());
                if project.starts_with(&expanded) || project == expanded {
                    for rule in rules {
                        if event_matches(&rule.event, &ctx.event) {
                            applicable_rules.push(rule);
                        }
                    }
                }
            }
        }

        // Evaluate each rule
        for rule in applicable_rules {
            // Check `when` condition
            if let Some(condition) = &rule.condition {
                if !eval_to_bool(condition, ctx) {
                    continue;
                }
            }

            // Check `unless` override
            if let Some(unless_expr) = &rule.unless {
                if eval_to_bool(unless_expr, ctx) {
                    continue;
                }
            }

            // Evaluate action
            match &rule.action {
                ParsedAction::Reject(expr) => {
                    if eval_to_bool(expr, ctx) {
                        let reason = rule
                            .reason
                            .clone()
                            .unwrap_or_else(|| format!("Rule '{}' rejected the action", rule.name));
                        return RuleAction::Reject(reason);
                    }
                }
                ParsedAction::Require(expr) => {
                    if !eval_to_bool(expr, ctx) {
                        let reason = rule
                            .reason
                            .clone()
                            .unwrap_or_else(|| format!("Rule '{}' requirement not met", rule.name));
                        return RuleAction::Reject(reason);
                    }
                }
                ParsedAction::Modify(expr) => {
                    let val = eval_to_string(expr, ctx);
                    return RuleAction::Modify(val);
                }
            }
        }

        RuleAction::Allow
    }

    pub fn rule_count(&self) -> usize {
        self.global_rules.len()
            + self.scoped_rules.iter().map(|(_, r)| r.len()).sum::<usize>()
    }

    /// Get a summary of active rules for the system prompt
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        for rule in &self.global_rules {
            let action_type = match &rule.action {
                ParsedAction::Reject(_) => "REJECT",
                ParsedAction::Require(_) => "REQUIRE",
                ParsedAction::Modify(_) => "MODIFY",
            };
            lines.push(format!(
                "- [{}] {} (on {:?}): {}",
                action_type,
                rule.name,
                rule.event,
                rule.reason.as_deref().unwrap_or("no reason")
            ));
        }
        for (path, rules) in &self.scoped_rules {
            for rule in rules {
                let action_type = match &rule.action {
                    ParsedAction::Reject(_) => "REJECT",
                    ParsedAction::Require(_) => "REQUIRE",
                    ParsedAction::Modify(_) => "MODIFY",
                };
                lines.push(format!(
                    "- [{}] {} (on {:?}, scope: {}): {}",
                    action_type,
                    rule.name,
                    rule.event,
                    path,
                    rule.reason.as_deref().unwrap_or("no reason")
                ));
            }
        }
        lines.join("\n")
    }
}

fn event_matches(rule_event: &Event, actual: &Event) -> bool {
    match (rule_event, actual) {
        (Event::Any, _) => true,
        (a, b) => a == b,
    }
}

fn eval_to_bool(expr: &Expression, ctx: &EvalContext) -> bool {
    match expr {
        Expression::BoolLit(b) => *b,
        Expression::Not(inner) => !eval_to_bool(inner, ctx),
        Expression::BinOp { left, op, right } => {
            match op {
                BinOperator::And => eval_to_bool(left, ctx) && eval_to_bool(right, ctx),
                BinOperator::Or => eval_to_bool(left, ctx) || eval_to_bool(right, ctx),
                BinOperator::Eq => eval_to_string(left, ctx) == eval_to_string(right, ctx),
                BinOperator::Neq => eval_to_string(left, ctx) != eval_to_string(right, ctx),
            }
        }
        Expression::Call { name, args } => eval_builtin_bool(name, args, ctx),
        Expression::Ident(name) => {
            match ctx.variables.get(name.as_str()) {
                Some(EvalValue::Bool(b)) => *b,
                Some(EvalValue::String(s)) => !s.is_empty(),
                _ => false,
            }
        }
        Expression::InExpr { value, list } => {
            let val = eval_to_string(value, ctx);
            match list.as_ref() {
                Expression::List(items) => {
                    items.iter().any(|item| eval_to_string(item, ctx) == val)
                }
                Expression::Ident(name) => {
                    match ctx.variables.get(name.as_str()) {
                        Some(EvalValue::List(items)) => items.iter().any(|s| *s == val),
                        _ => false,
                    }
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn eval_to_string(expr: &Expression, ctx: &EvalContext) -> String {
    match expr {
        Expression::StringLit(s) => s.clone(),
        Expression::NumberLit(n) => n.to_string(),
        Expression::BoolLit(b) => b.to_string(),
        Expression::Ident(name) => {
            match ctx.variables.get(name.as_str()) {
                Some(EvalValue::String(s)) => s.clone(),
                Some(EvalValue::Number(n)) => n.to_string(),
                Some(EvalValue::Bool(b)) => b.to_string(),
                _ => String::new(),
            }
        }
        Expression::Call { name, args } => {
            // Some builtins return strings
            match name.as_str() {
                "extension" => {
                    let path = args.first().map(|a| eval_to_string(a, ctx)).unwrap_or_default();
                    builtins::builtin_extension(&path)
                }
                "dirname" => {
                    let path = args.first().map(|a| eval_to_string(a, ctx)).unwrap_or_default();
                    builtins::builtin_dirname(&path)
                }
                _ => eval_builtin_bool(name, args, ctx).to_string(),
            }
        }
        _ => String::new(),
    }
}

fn eval_builtin_bool(name: &str, args: &[Expression], ctx: &EvalContext) -> bool {
    match name {
        "contains" => {
            let haystack = args.first().map(|a| eval_to_string(a, ctx)).unwrap_or_default();
            let needle = args.get(1).map(|a| eval_to_string(a, ctx)).unwrap_or_default();
            builtins::builtin_contains(&haystack, &needle)
        }
        "matches" => {
            let text = args.first().map(|a| eval_to_string(a, ctx)).unwrap_or_default();
            let pattern = args.get(1).map(|a| eval_to_string(a, ctx)).unwrap_or_default();
            builtins::builtin_matches(&text, &pattern)
        }
        "files_exist" => {
            let path = args.first().map(|a| eval_to_string(a, ctx)).unwrap_or_default();
            builtins::builtin_files_exist(&path)
        }
        "files_match" => {
            let pattern = args.first().map(|a| eval_to_string(a, ctx)).unwrap_or_default();
            // Get files list from second arg or from context
            let files = if let Some(arg) = args.get(1) {
                match arg {
                    Expression::Ident(name) => {
                        match ctx.variables.get(name.as_str()) {
                            Some(EvalValue::List(list)) => list.clone(),
                            _ => vec![],
                        }
                    }
                    _ => vec![eval_to_string(arg, ctx)],
                }
            } else {
                match ctx.variables.get("staged_files") {
                    Some(EvalValue::List(list)) => list.clone(),
                    _ => vec![],
                }
            };
            builtins::builtin_files_match(&pattern, &files)
        }
        "line_count" => {
            let path = args.first().map(|a| eval_to_string(a, ctx)).unwrap_or_default();
            let count = builtins::builtin_line_count(&path);
            // Used as boolean: line_count > 0
            count > 0
        }
        "adds_lines_matching" => {
            let pattern = args.first().map(|a| eval_to_string(a, ctx)).unwrap_or_default();
            let diff = ctx
                .variables
                .get("diff")
                .map(|v| match v {
                    EvalValue::String(s) => s.clone(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            builtins::builtin_adds_lines_matching(&pattern, &diff)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_reject_rule() {
        let mut engine = RulesEngine::new();
        engine
            .load(r#"
rule "no-co-author" {
  on commit
  reject contains(message, "Co-Authored-By")
  reason "No co-author lines"
}
"#)
            .unwrap();

        let mut ctx = EvalContext::new(Event::Commit);
        ctx.set_str("message", "fix bug\n\nCo-Authored-By: someone");

        assert_eq!(
            engine.evaluate(&ctx, None),
            RuleAction::Reject("No co-author lines".to_string())
        );
    }

    #[test]
    fn test_reject_does_not_fire() {
        let mut engine = RulesEngine::new();
        engine
            .load(r#"
rule "no-co-author" {
  on commit
  reject contains(message, "Co-Authored-By")
}
"#)
            .unwrap();

        let mut ctx = EvalContext::new(Event::Commit);
        ctx.set_str("message", "clean commit message");

        assert_eq!(engine.evaluate(&ctx, None), RuleAction::Allow);
    }

    #[test]
    fn test_require_rule() {
        let mut engine = RulesEngine::new();
        engine
            .load(r#"
rule "need-tests" {
  on commit
  require files_match("*test*", staged_files)
  reason "Must include test files"
}
"#)
            .unwrap();

        let mut ctx = EvalContext::new(Event::Commit);
        ctx.set("staged_files", EvalValue::List(vec!["main.rs".to_string()]));

        assert_eq!(
            engine.evaluate(&ctx, None),
            RuleAction::Reject("Must include test files".to_string())
        );

        // Now with test files
        ctx.set(
            "staged_files",
            EvalValue::List(vec!["main.rs".to_string(), "test_main.rs".to_string()]),
        );

        assert_eq!(engine.evaluate(&ctx, None), RuleAction::Allow);
    }

    #[test]
    fn test_unless_override() {
        let mut engine = RulesEngine::new();
        engine
            .load(r#"
rule "block-rm" {
  on tool:bash
  reject matches(command, "rm -rf")
  unless confirmed_by_user
  reason "Blocked"
}
"#)
            .unwrap();

        let mut ctx = EvalContext::new(Event::Tool("bash".to_string()));
        ctx.set_str("command", "rm -rf /tmp/junk");
        ctx.set("confirmed_by_user", EvalValue::Bool(false));

        assert_eq!(
            engine.evaluate(&ctx, None),
            RuleAction::Reject("Blocked".to_string())
        );

        // With user confirmation
        ctx.set("confirmed_by_user", EvalValue::Bool(true));
        assert_eq!(engine.evaluate(&ctx, None), RuleAction::Allow);
    }

    #[test]
    fn test_scoped_rules() {
        let mut engine = RulesEngine::new();
        engine
            .load(r#"
scope "~/Developer/Serena" {
  rule "swift-only" {
    on tool:file_write
    require extension(path) == "swift"
    reason "Only Swift files in Serena"
  }
}
"#)
            .unwrap();

        let home = dirs::home_dir().unwrap().to_string_lossy().to_string();
        let serena_path = format!("{home}/Developer/Serena");

        let mut ctx = EvalContext::new(Event::Tool("file_write".to_string()));
        ctx.set_str("path", "test.py");

        // In Serena project — rule fires
        assert_eq!(
            engine.evaluate(&ctx, Some(&serena_path)),
            RuleAction::Reject("Only Swift files in Serena".to_string())
        );

        // Different project — rule doesn't apply
        assert_eq!(
            engine.evaluate(&ctx, Some("/other/project")),
            RuleAction::Allow
        );
    }

    #[test]
    fn test_when_condition() {
        let mut engine = RulesEngine::new();
        engine
            .load(r#"
rule "test-rule" {
  on commit
  when project in ["Serena", "FolkOS"]
  reject contains(message, "WIP")
  reason "No WIP commits in core projects"
}
"#)
            .unwrap();

        let mut ctx = EvalContext::new(Event::Commit);
        ctx.set_str("message", "WIP: fixing something");
        ctx.set_str("project", "Serena");

        assert_eq!(
            engine.evaluate(&ctx, None),
            RuleAction::Reject("No WIP commits in core projects".to_string())
        );

        // Different project — when condition fails
        ctx.set_str("project", "random-project");
        assert_eq!(engine.evaluate(&ctx, None), RuleAction::Allow);
    }

    #[test]
    fn test_event_matching() {
        let mut engine = RulesEngine::new();
        engine
            .load(r#"
rule "commit-only" {
  on commit
  reject false
}
"#)
            .unwrap();

        // Commit event — rule is evaluated
        let ctx = EvalContext::new(Event::Commit);
        assert_eq!(engine.evaluate(&ctx, None), RuleAction::Allow);

        // Different event — rule not evaluated
        let ctx = EvalContext::new(Event::Tool("bash".to_string()));
        assert_eq!(engine.evaluate(&ctx, None), RuleAction::Allow);
    }

    #[test]
    fn test_multiple_rules() {
        let mut engine = RulesEngine::new();
        engine
            .load(r#"
rule "a" {
  on commit
  reject contains(message, "bad")
  reason "Rule A"
}

rule "b" {
  on commit
  reject contains(message, "terrible")
  reason "Rule B"
}
"#)
            .unwrap();

        let mut ctx = EvalContext::new(Event::Commit);
        ctx.set_str("message", "this is terrible");

        // Rule B should fire (rule A passes)
        assert_eq!(
            engine.evaluate(&ctx, None),
            RuleAction::Reject("Rule B".to_string())
        );
    }

    #[test]
    fn test_rule_count() {
        let mut engine = RulesEngine::new();
        engine
            .load(r#"
rule "a" { on commit reject false }
rule "b" { on commit reject false }
scope "~/test" {
  rule "c" { on commit reject false }
}
"#)
            .unwrap();

        assert_eq!(engine.rule_count(), 3);
    }

    #[test]
    fn test_summary() {
        let mut engine = RulesEngine::new();
        engine
            .load(r#"
rule "no-wip" {
  on commit
  reject contains(message, "WIP")
  reason "No WIP commits"
}
"#)
            .unwrap();

        let summary = engine.summary();
        assert!(summary.contains("no-wip"));
        assert!(summary.contains("REJECT"));
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[test]
    fn test_security_regex_catastrophic_backtracking() {
        // P0 security red test
        // A regex pattern designed for catastrophic backtracking must not hang
        // the matches() builtin. The builtin uses regex::Regex which has linear-time
        // guarantees, but we verify the engine doesn't hang.
        let mut engine = RulesEngine::new();
        engine
            .load(r#"
rule "evil-regex" {
  on commit
  reject matches(message, "(a+)+b")
  reason "Blocked"
}
"#)
            .unwrap();

        let mut ctx = EvalContext::new(Event::Commit);
        // Input designed to trigger catastrophic backtracking in naive engines
        ctx.set_str("message", &"a".repeat(30));

        // Should complete quickly — regex crate has linear-time guarantee
        let result = engine.evaluate(&ctx, None);
        // The pattern won't match (no trailing 'b'), so rule passes
        assert_eq!(result, RuleAction::Allow);
    }

    #[test]
    fn test_security_extremely_long_string_literals() {
        // P0 security red test
        // Rules with very long string literals must not crash the parser or evaluator
        let long_string = "x".repeat(100_000);
        let rule_text = format!(
            r#"
rule "long-rule" {{
  on commit
  reject contains(message, "{long_string}")
  reason "Blocked"
}}
"#
        );
        let mut engine = RulesEngine::new();
        let result = engine.load(&rule_text);
        // Should either parse successfully or return a clean error
        if let Ok(_) = result {
            let mut ctx = EvalContext::new(Event::Commit);
            ctx.set_str("message", "short");
            let action = engine.evaluate(&ctx, None);
            assert_eq!(action, RuleAction::Allow);
        }
        // No panic = pass
    }

    #[test]
    fn test_security_deeply_nested_boolean_no_stack_overflow() {
        // P0 security red test
        // Deeply nested boolean expressions in the evaluator must not stack overflow.
        // We build a deeply nested Expression tree manually since the parser may
        // limit nesting depth.
        let mut expr: Expression = Expression::BoolLit(true);
        for _ in 0..100 {
            expr = Expression::BinOp {
                left: Box::new(expr),
                op: BinOperator::And,
                right: Box::new(Expression::BoolLit(true)),
            };
        }
        let ctx = EvalContext::new(Event::Commit);
        let result = eval_to_bool(&expr, &ctx);
        assert!(result, "Deeply nested AND(true, true, ...) should evaluate to true");
    }

    #[test]
    fn test_security_rule_name_with_brace_injection() {
        // P0 security red test
        // A rule "name" containing `}` must not break the parser
        let rule_text = r#"
rule "name-with-}-brace" {
  on commit
  reject false
}
"#;
        let mut engine = RulesEngine::new();
        let result = engine.load(rule_text);
        // Parser may reject this or handle it — either is fine, no panic
        // If it parses, the rule name should be preserved
        if let Ok(count) = result {
            assert!(count >= 1);
        }
        // No panic = pass
    }
}
