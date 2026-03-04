use super::lexer::{Token, TokenKind, Lexer};

/// A set of rules and scopes
#[derive(Debug, Clone)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
    pub scopes: Vec<Scope>,
}

/// A single rule definition
#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    pub event: Event,
    pub condition: Option<Expression>,    // when
    pub action: RuleAction,               // reject or require
    pub unless: Option<Expression>,       // override escape hatch
    pub reason: Option<String>,
}

/// A scoped group of rules
#[derive(Debug, Clone)]
pub struct Scope {
    pub path: String,
    pub rules: Vec<Rule>,
}

/// What triggers the rule
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Commit,
    PrCreate,
    Tool(String),       // tool:bash, tool:file_write, etc.
    Response,
    SessionStart,
    Any,
}

/// The enforcement action
#[derive(Debug, Clone)]
pub enum RuleAction {
    Reject(Expression),
    Require(Expression),
    Modify(Expression),
}

/// Expression AST
#[derive(Debug, Clone)]
pub enum Expression {
    /// Function call: contains(message, "Co-Authored-By")
    Call { name: String, args: Vec<Expression> },
    /// String literal
    StringLit(String),
    /// Number literal
    NumberLit(f64),
    /// Boolean literal
    BoolLit(bool),
    /// Identifier (variable reference)
    Ident(String),
    /// Binary operation: a && b, a || b, a == b, a != b
    BinOp { left: Box<Expression>, op: BinOperator, right: Box<Expression> },
    /// Unary not: !expr
    Not(Box<Expression>),
    /// List literal: ["Serena", "FolkOS"]
    List(Vec<Expression>),
    /// In expression: x in [...]
    InExpr { value: Box<Expression>, list: Box<Expression> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOperator {
    And,
    Or,
    Eq,
    Neq,
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn parse(input: &str) -> Result<RuleSet, String> {
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser { tokens, pos: 0 };
        parser.parse_ruleset()
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos.min(self.tokens.len() - 1)];
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: &TokenKind) -> Result<(), String> {
        let tok = self.current().clone();
        if std::mem::discriminant(&tok.kind) == std::mem::discriminant(expected) {
            self.advance();
            Ok(())
        } else {
            Err(format!(
                "Expected {:?}, got {:?} at {}:{}",
                expected, tok.kind, tok.line, tok.col
            ))
        }
    }

    fn parse_ruleset(&mut self) -> Result<RuleSet, String> {
        let mut rules = Vec::new();
        let mut scopes = Vec::new();

        while self.current().kind != TokenKind::Eof {
            match &self.current().kind {
                TokenKind::Rule => {
                    rules.push(self.parse_rule()?);
                }
                TokenKind::Scope => {
                    scopes.push(self.parse_scope()?);
                }
                _ => {
                    return Err(format!(
                        "Expected 'rule' or 'scope', got {:?} at {}:{}",
                        self.current().kind,
                        self.current().line,
                        self.current().col
                    ));
                }
            }
        }

        Ok(RuleSet { rules, scopes })
    }

    fn parse_rule(&mut self) -> Result<Rule, String> {
        self.expect(&TokenKind::Rule)?;

        // Rule name
        let name = match &self.current().kind {
            TokenKind::String(s) => {
                let s = s.clone();
                self.advance();
                s
            }
            _ => return Err(format!("Expected rule name string at {}:{}", self.current().line, self.current().col)),
        };

        self.expect(&TokenKind::LBrace)?;

        let mut event = Event::Any;
        let mut condition = None;
        let mut action = None;
        let mut unless = None;
        let mut reason = None;

        while self.current().kind != TokenKind::RBrace && self.current().kind != TokenKind::Eof {
            match &self.current().kind {
                TokenKind::On => {
                    self.advance();
                    event = self.parse_event()?;
                }
                TokenKind::When => {
                    self.advance();
                    condition = Some(self.parse_expression()?);
                }
                TokenKind::Reject => {
                    self.advance();
                    action = Some(RuleAction::Reject(self.parse_expression()?));
                }
                TokenKind::Require => {
                    self.advance();
                    action = Some(RuleAction::Require(self.parse_expression()?));
                }
                TokenKind::Modify => {
                    self.advance();
                    action = Some(RuleAction::Modify(self.parse_expression()?));
                }
                TokenKind::Unless => {
                    self.advance();
                    unless = Some(self.parse_expression()?);
                }
                TokenKind::Reason => {
                    self.advance();
                    if let TokenKind::String(s) = &self.current().kind {
                        reason = Some(s.clone());
                        self.advance();
                    }
                }
                _ => {
                    return Err(format!(
                        "Unexpected token {:?} in rule body at {}:{}",
                        self.current().kind,
                        self.current().line,
                        self.current().col
                    ));
                }
            }
        }

        self.expect(&TokenKind::RBrace)?;

        let action = action.ok_or_else(|| format!("Rule '{name}' has no reject/require/modify action"))?;

        Ok(Rule {
            name,
            event,
            condition,
            action,
            unless,
            reason,
        })
    }

    fn parse_scope(&mut self) -> Result<Scope, String> {
        self.expect(&TokenKind::Scope)?;

        let path = match &self.current().kind {
            TokenKind::String(s) => {
                let s = s.clone();
                self.advance();
                s
            }
            _ => return Err("Expected scope path string".to_string()),
        };

        self.expect(&TokenKind::LBrace)?;

        let mut rules = Vec::new();
        while self.current().kind != TokenKind::RBrace && self.current().kind != TokenKind::Eof {
            rules.push(self.parse_rule()?);
        }

        self.expect(&TokenKind::RBrace)?;

        Ok(Scope { path, rules })
    }

    fn parse_event(&mut self) -> Result<Event, String> {
        match &self.current().kind {
            TokenKind::Ident(s) => {
                let s = s.clone();
                self.advance();

                // Handle "tool:name" split across tokens: tool + : + name
                if s == "tool" && self.current().kind == TokenKind::Colon {
                    self.advance(); // skip :
                    if let TokenKind::Ident(tool_name) = &self.current().kind {
                        let tool_name = tool_name.clone();
                        self.advance();
                        return Ok(Event::Tool(tool_name));
                    }
                }

                match s.as_str() {
                    "commit" => Ok(Event::Commit),
                    "pr_create" => Ok(Event::PrCreate),
                    "response" => Ok(Event::Response),
                    "session_start" => Ok(Event::SessionStart),
                    s if s.starts_with("tool:") => Ok(Event::Tool(s[5..].to_string())),
                    _ => Ok(Event::Tool(s.to_string())),
                }
            }
            _ => Err(format!(
                "Expected event name at {}:{}",
                self.current().line,
                self.current().col
            )),
        }
    }

    fn parse_expression(&mut self) -> Result<Expression, String> {
        let left = self.parse_unary()?;
        self.parse_binary(left, 0)
    }

    fn parse_binary(&mut self, left: Expression, min_prec: u8) -> Result<Expression, String> {
        let mut result = left;

        loop {
            let (op, prec) = match &self.current().kind {
                TokenKind::Pipe | TokenKind::Or => (BinOperator::Or, 1),
                TokenKind::Ampersand | TokenKind::And => (BinOperator::And, 2),
                TokenKind::Eq => (BinOperator::Eq, 3),
                TokenKind::Neq => (BinOperator::Neq, 3),
                TokenKind::In => {
                    self.advance();
                    let list = self.parse_primary()?;
                    result = Expression::InExpr {
                        value: Box::new(result),
                        list: Box::new(list),
                    };
                    continue;
                }
                _ => break,
            };

            if prec < min_prec {
                break;
            }

            self.advance();
            let right = self.parse_unary()?;
            let right = self.parse_binary(right, prec + 1)?;

            result = Expression::BinOp {
                left: Box::new(result),
                op,
                right: Box::new(right),
            };
        }

        Ok(result)
    }

    fn parse_unary(&mut self) -> Result<Expression, String> {
        if self.current().kind == TokenKind::Bang || self.current().kind == TokenKind::Not {
            self.advance();
            let expr = self.parse_primary()?;
            return Ok(Expression::Not(Box::new(expr)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expression, String> {
        match &self.current().kind.clone() {
            TokenKind::String(s) => {
                let s = s.clone();
                self.advance();
                Ok(Expression::StringLit(s))
            }
            TokenKind::Number(n) => {
                let n = *n;
                self.advance();
                Ok(Expression::NumberLit(n))
            }
            TokenKind::Bool(b) => {
                let b = *b;
                self.advance();
                Ok(Expression::BoolLit(b))
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();

                // Check for function call
                if self.current().kind == TokenKind::LParen {
                    self.advance(); // skip (
                    let mut args = Vec::new();
                    while self.current().kind != TokenKind::RParen
                        && self.current().kind != TokenKind::Eof
                    {
                        args.push(self.parse_expression()?);
                        if self.current().kind == TokenKind::Comma {
                            self.advance();
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expression::Call { name, args })
                } else {
                    Ok(Expression::Ident(name))
                }
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(&TokenKind::RParen)?;
                Ok(expr)
            }
            TokenKind::LBracket => {
                self.advance();
                let mut items = Vec::new();
                while self.current().kind != TokenKind::RBracket
                    && self.current().kind != TokenKind::Eof
                {
                    items.push(self.parse_expression()?);
                    if self.current().kind == TokenKind::Comma {
                        self.advance();
                    }
                }
                self.expect(&TokenKind::RBracket)?;
                Ok(Expression::List(items))
            }
            _ => Err(format!(
                "Unexpected token {:?} at {}:{}",
                self.current().kind,
                self.current().line,
                self.current().col
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_rule() {
        let input = r#"
rule "no-co-author" {
  on commit
  reject contains(message, "Co-Authored-By")
  reason "Never add co-author lines"
}
"#;
        let ruleset = Parser::parse(input).unwrap();
        assert_eq!(ruleset.rules.len(), 1);
        assert_eq!(ruleset.rules[0].name, "no-co-author");
        assert_eq!(ruleset.rules[0].event, Event::Commit);
        assert!(ruleset.rules[0].reason.is_some());
    }

    #[test]
    fn test_parse_rule_with_condition() {
        let input = r#"
rule "security-tests" {
  on commit
  when project in ["Serena", "FolkOS"]
  require files_match("*RedTests*") in staged_files
  reason "Security red tests required"
}
"#;
        let ruleset = Parser::parse(input).unwrap();
        assert_eq!(ruleset.rules.len(), 1);
        assert!(ruleset.rules[0].condition.is_some());
    }

    #[test]
    fn test_parse_tool_event() {
        let input = r#"
rule "block-destructive" {
  on tool:bash
  reject matches(command, "rm -rf")
  unless confirmed_by_user
  reason "Destructive commands need confirmation"
}
"#;
        let ruleset = Parser::parse(input).unwrap();
        assert_eq!(ruleset.rules[0].event, Event::Tool("bash".to_string()));
        assert!(ruleset.rules[0].unless.is_some());
    }

    #[test]
    fn test_parse_scope() {
        let input = r#"
scope "~/Developer/Serena" {
  rule "swift-conventions" {
    on tool:file_write
    require !contains(content, "force try")
    reason "No force try in Serena"
  }
}
"#;
        let ruleset = Parser::parse(input).unwrap();
        assert_eq!(ruleset.scopes.len(), 1);
        assert_eq!(ruleset.scopes[0].path, "~/Developer/Serena");
        assert_eq!(ruleset.scopes[0].rules.len(), 1);
    }

    #[test]
    fn test_parse_boolean_expressions() {
        let input = r#"
rule "complex" {
  on commit
  reject contains(message, "fixup") || contains(message, "squash")
  reason "No fixup commits"
}
"#;
        let ruleset = Parser::parse(input).unwrap();
        if let RuleAction::Reject(Expression::BinOp { op, .. }) = &ruleset.rules[0].action {
            assert_eq!(*op, BinOperator::Or);
        } else {
            panic!("Expected BinOp with Or");
        }
    }

    #[test]
    fn test_parse_multiple_rules() {
        let input = r#"
rule "a" {
  on commit
  reject false
}

rule "b" {
  on tool:bash
  require true
}
"#;
        let ruleset = Parser::parse(input).unwrap();
        assert_eq!(ruleset.rules.len(), 2);
    }

    #[test]
    fn test_parse_error_missing_action() {
        let input = r#"
rule "bad" {
  on commit
  reason "no action"
}
"#;
        let result = Parser::parse(input);
        assert!(result.is_err());
    }
}
