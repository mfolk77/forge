use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords
    Rule,
    Scope,
    On,
    When,
    Reject,
    Require,
    Modify,
    Unless,
    Reason,
    In,
    And,
    Or,
    Not,

    // Literals
    String(String),
    Ident(String),
    Number(f64),
    Bool(bool),

    // Symbols
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Eq,
    Neq,
    Pipe,
    Ampersand,
    Bang,
    Colon,

    // Special
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} at {}:{}", self.kind, self.line, self.col)
    }
}

pub struct Lexer {
    input: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();

        loop {
            self.skip_whitespace_and_comments();

            if self.pos >= self.input.len() {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    line: self.line,
                    col: self.col,
                });
                break;
            }

            let token = self.next_token()?;
            tokens.push(token);
        }

        Ok(tokens)
    }

    fn skip_whitespace_and_comments(&mut self) {
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            if ch == '#' {
                // Skip to end of line
                while self.pos < self.input.len() && self.input[self.pos] != '\n' {
                    self.pos += 1;
                }
            } else if ch == '\n' {
                self.pos += 1;
                self.line += 1;
                self.col = 1;
            } else if ch.is_whitespace() {
                self.pos += 1;
                self.col += 1;
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, String> {
        let line = self.line;
        let col = self.col;
        let ch = self.input[self.pos];

        match ch {
            '{' => { self.advance(); Ok(Token { kind: TokenKind::LBrace, line, col }) }
            '}' => { self.advance(); Ok(Token { kind: TokenKind::RBrace, line, col }) }
            '(' => { self.advance(); Ok(Token { kind: TokenKind::LParen, line, col }) }
            ')' => { self.advance(); Ok(Token { kind: TokenKind::RParen, line, col }) }
            '[' => { self.advance(); Ok(Token { kind: TokenKind::LBracket, line, col }) }
            ']' => { self.advance(); Ok(Token { kind: TokenKind::RBracket, line, col }) }
            ',' => { self.advance(); Ok(Token { kind: TokenKind::Comma, line, col }) }
            '.' => { self.advance(); Ok(Token { kind: TokenKind::Dot, line, col }) }
            ':' => { self.advance(); Ok(Token { kind: TokenKind::Colon, line, col }) }
            '!' => {
                self.advance();
                if self.pos < self.input.len() && self.input[self.pos] == '=' {
                    self.advance();
                    Ok(Token { kind: TokenKind::Neq, line, col })
                } else {
                    Ok(Token { kind: TokenKind::Bang, line, col })
                }
            }
            '=' => {
                self.advance();
                if self.pos < self.input.len() && self.input[self.pos] == '=' {
                    self.advance();
                }
                Ok(Token { kind: TokenKind::Eq, line, col })
            }
            '|' => {
                self.advance();
                if self.pos < self.input.len() && self.input[self.pos] == '|' {
                    self.advance();
                }
                Ok(Token { kind: TokenKind::Pipe, line, col })
            }
            '&' => {
                self.advance();
                if self.pos < self.input.len() && self.input[self.pos] == '&' {
                    self.advance();
                }
                Ok(Token { kind: TokenKind::Ampersand, line, col })
            }
            '"' => self.read_string(line, col),
            c if c.is_ascii_digit() => self.read_number(line, col),
            c if c.is_alphanumeric() || c == '_' || c == '~' || c == '/' || c == '*' => {
                self.read_ident_or_keyword(line, col)
            }
            _ => {
                self.advance();
                Err(format!("Unexpected character '{}' at {}:{}", ch, line, col))
            }
        }
    }

    fn advance(&mut self) {
        self.pos += 1;
        self.col += 1;
    }

    fn read_string(&mut self, line: usize, col: usize) -> Result<Token, String> {
        self.advance(); // skip opening quote
        let mut s = String::new();

        while self.pos < self.input.len() && self.input[self.pos] != '"' {
            if self.input[self.pos] == '\\' && self.pos + 1 < self.input.len() {
                self.advance();
                match self.input[self.pos] {
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    '"' => s.push('"'),
                    '\\' => s.push('\\'),
                    c => s.push(c),
                }
            } else {
                s.push(self.input[self.pos]);
            }
            self.advance();
        }

        if self.pos >= self.input.len() {
            return Err(format!("Unterminated string at {line}:{col}"));
        }

        self.advance(); // skip closing quote
        Ok(Token { kind: TokenKind::String(s), line, col })
    }

    fn read_number(&mut self, line: usize, col: usize) -> Result<Token, String> {
        let mut s = String::new();
        while self.pos < self.input.len()
            && (self.input[self.pos].is_ascii_digit() || self.input[self.pos] == '.')
        {
            s.push(self.input[self.pos]);
            self.advance();
        }
        let num: f64 = s.parse().map_err(|e| format!("Invalid number at {line}:{col}: {e}"))?;
        Ok(Token { kind: TokenKind::Number(num), line, col })
    }

    fn read_ident_or_keyword(&mut self, line: usize, col: usize) -> Result<Token, String> {
        let mut s = String::new();
        while self.pos < self.input.len() {
            let c = self.input[self.pos];
            if c.is_alphanumeric() || c == '_' || c == '~' || c == '/' || c == '*' || c == '.' || c == '-' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }

        let kind = match s.as_str() {
            "rule" => TokenKind::Rule,
            "scope" => TokenKind::Scope,
            "on" => TokenKind::On,
            "when" => TokenKind::When,
            "reject" => TokenKind::Reject,
            "require" => TokenKind::Require,
            "modify" => TokenKind::Modify,
            "unless" => TokenKind::Unless,
            "reason" => TokenKind::Reason,
            "in" => TokenKind::In,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            "not" => TokenKind::Not,
            "true" => TokenKind::Bool(true),
            "false" => TokenKind::Bool(false),
            _ => TokenKind::Ident(s),
        };

        Ok(Token { kind, line, col })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_rule() {
        let input = r#"rule "no-co-author" {
  on commit
  reject contains(message, "Co-Authored-By")
  reason "Never add co-author lines"
}"#;
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize().unwrap();

        assert_eq!(tokens[0].kind, TokenKind::Rule);
        assert_eq!(tokens[1].kind, TokenKind::String("no-co-author".to_string()));
        assert_eq!(tokens[2].kind, TokenKind::LBrace);
        assert_eq!(tokens[3].kind, TokenKind::On);
    }

    #[test]
    fn test_comments() {
        let input = "# this is a comment\nrule";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Rule);
    }

    #[test]
    fn test_operators() {
        let input = "== != || && !";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Eq);
        assert_eq!(tokens[1].kind, TokenKind::Neq);
        assert_eq!(tokens[2].kind, TokenKind::Pipe);
        assert_eq!(tokens[3].kind, TokenKind::Ampersand);
        assert_eq!(tokens[4].kind, TokenKind::Bang);
    }

    #[test]
    fn test_string_escapes() {
        let input = r#""hello \"world\" \n""#;
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize().unwrap();
        if let TokenKind::String(s) = &tokens[0].kind {
            assert!(s.contains('"'));
            assert!(s.contains('\n'));
        } else {
            panic!("Expected string token");
        }
    }

    #[test]
    fn test_scope() {
        let input = r#"scope "~/Developer/Serena" { }"#;
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Scope);
        assert_eq!(tokens[1].kind, TokenKind::String("~/Developer/Serena".to_string()));
    }
}
