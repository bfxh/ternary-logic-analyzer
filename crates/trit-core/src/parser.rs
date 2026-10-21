//! 表达式解析器：递归下降，把文本解析为 [`Expr`]。
//!
//! 解析错误带行列位置，便于 CLI 给出可读诊断。

use crate::ast::Expr;
use crate::value::Ternary;

/// 解析错误类型。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// 表达式意外结束。
    #[error("表达式在第 {line} 行第 {column} 列意外结束")]
    UnexpectedEof {
        /// 行号（1 起）。
        line: usize,
        /// 列号（1 起）。
        column: usize,
    },
    /// 遇到无法识别的字符。
    #[error("第 {line} 行第 {column} 列出现无法识别的字符 {ch:?}")]
    UnexpectedChar {
        /// 行号（1 起）。
        line: usize,
        /// 列号（1 起）。
        column: usize,
        /// 实际字符。
        ch: char,
    },
    /// 括号不匹配。
    #[error("第 {line} 行第 {column} 列缺少右括号 ')'")]
    UnclosedParen {
        /// 行号（1 起）。
        line: usize,
        /// 列号（1 起）。
        column: usize,
    },
    /// 标识符非法（空或含非法字符）。
    #[error("第 {line} 行第 {column} 列的标识符不合法：{name:?}")]
    InvalidIdentifier {
        /// 行号（1 起）。
        line: usize,
        /// 列号（1 起）。
        column: usize,
        /// 标识符原文。
        name: String,
    },
    /// 表达式为空。
    #[error("表达式为空")]
    Empty,
}

/// 递归下降解析器。
pub struct Parser {
    input: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}

impl Parser {
    /// 从字符串创建解析器。
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    /// 解析完整表达式，要求输入被完全消费。
    pub fn parse(input: &str) -> Result<Expr, ParseError> {
        let mut p = Parser::new(input);
        p.skip_ws();
        if p.peek().is_none() {
            return Err(ParseError::Empty);
        }
        let expr = p.parse_equiv()?;
        if let Some(ch) = p.peek() {
            return Err(ParseError::UnexpectedChar {
                line: p.line,
                column: p.column,
                ch,
            });
        }
        Ok(expr)
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.input.get(self.pos).copied()?;
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn skip_ws(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.bump();
            } else {
                break;
            }
        }
    }

    /// 尝试匹配一个运算符关键字（不区分 ASCII 空格要求）。
    fn eat_op(&mut self, ops: &[&str]) -> bool {
        self.skip_ws();
        for op in ops {
            let chars: Vec<char> = op.chars().collect();
            if self.input.len() >= self.pos + chars.len()
                && self.input[self.pos..self.pos + chars.len()] == chars[..]
            {
                for _ in 0..chars.len() {
                    self.bump();
                }
                return true;
            }
        }
        false
    }

    fn parse_equiv(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_implies()?;
        loop {
            self.skip_ws();
            if self.eat_op(&["<=>", "≡", "=="]) {
                let right = self.parse_implies()?;
                left = Expr::equiv(left, right);
            } else {
                return Ok(left);
            }
        }
    }

    fn parse_implies(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_or()?;
        self.skip_ws();
        if self.eat_op(&["=>", "→", "⇒"]) {
            // 右结合
            let right = self.parse_implies()?;
            return Ok(Expr::implies(left, right));
        }
        Ok(left)
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_xor()?;
        loop {
            self.skip_ws();
            if self.eat_op(&["||", "∨", "|"]) {
                let right = self.parse_xor()?;
                left = Expr::or(left, right);
            } else {
                return Ok(left);
            }
        }
    }

    fn parse_xor(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        loop {
            self.skip_ws();
            if self.eat_op(&["⊕", "^"]) {
                let right = self.parse_and()?;
                left = Expr::xor(left, right);
            } else {
                return Ok(left);
            }
        }
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            self.skip_ws();
            if self.eat_op(&["&&", "∧", "&"]) {
                let right = self.parse_unary()?;
                left = Expr::and(left, right);
            } else {
                return Ok(left);
            }
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();
        if self.eat_op(&["!", "~", "¬"]) {
            let inner = self.parse_unary()?;
            return Ok(Expr::logical_not(inner));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();
        let Some(ch) = self.peek() else {
            return Err(ParseError::UnexpectedEof {
                line: self.line,
                column: self.column,
            });
        };

        if ch == '(' {
            self.bump();
            let inner = self.parse_equiv()?;
            self.skip_ws();
            if self.peek() != Some(')') {
                return Err(ParseError::UnclosedParen {
                    line: self.line,
                    column: self.column,
                });
            }
            self.bump();
            return Ok(inner);
        }

        if ch == ')' {
            return Err(ParseError::UnexpectedChar {
                line: self.line,
                column: self.column,
                ch,
            });
        }

        if is_ident_start(ch) {
            let start_line = self.line;
            let start_col = self.column;
            let mut name = String::new();
            while let Some(c) = self.peek() {
                if is_ident_char(c) {
                    name.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
            if name.is_empty() {
                return Err(ParseError::InvalidIdentifier {
                    line: start_line,
                    column: start_col,
                    name,
                });
            }
            return Ok(match name.as_str() {
                "0" | "False" | "false" | "FALSE" => Expr::Const(Ternary::False),
                "1" | "True" | "true" | "TRUE" => Expr::Const(Ternary::True),
                "X" | "x" | "?" | "Unknown" | "unknown" => Expr::Const(Ternary::Unknown),
                _ => Expr::Var(name),
            });
        }

        if let Some(v) = Ternary::parse_char(ch) {
            self.bump();
            return Ok(Expr::Const(v));
        }

        Err(ParseError::UnexpectedChar {
            line: self.line,
            column: self.column,
            ch,
        })
    }
}

/// 标识符首字符：字母或下划线。
fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// 标识符后续字符：字母、数字或下划线。
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_variables() {
        assert_eq!(Parser::parse("a").unwrap(), Expr::Var("a".into()));
        assert_eq!(Parser::parse("a_1").unwrap(), Expr::Var("a_1".into()));
    }

    #[test]
    fn parses_constants() {
        assert_eq!(Parser::parse("0").unwrap(), Expr::Const(Ternary::False));
        assert_eq!(Parser::parse("1").unwrap(), Expr::Const(Ternary::True));
        assert_eq!(Parser::parse("X").unwrap(), Expr::Const(Ternary::Unknown));
        assert_eq!(Parser::parse("?").unwrap(), Expr::Const(Ternary::Unknown));
    }

    #[test]
    fn operator_precedence() {
        // a ∧ b ∨ c  =>  (a ∧ b) ∨ c
        let e = Parser::parse("a & b | c").unwrap();
        assert_eq!(
            e,
            Expr::or(
                Expr::and(Expr::Var("a".into()), Expr::Var("b".into())),
                Expr::Var("c".into())
            )
        );
    }

    #[test]
    fn not_binds_tighter_than_and() {
        let e = Parser::parse("!a & b").unwrap();
        assert_eq!(
            e,
            Expr::and(
                Expr::logical_not(Expr::Var("a".into())),
                Expr::Var("b".into())
            )
        );
    }

    #[test]
    fn implies_is_right_associative() {
        let e = Parser::parse("a => b => c").unwrap();
        assert_eq!(
            e,
            Expr::implies(
                Expr::Var("a".into()),
                Expr::implies(Expr::Var("b".into()), Expr::Var("c".into()))
            )
        );
    }

    #[test]
    fn parentheses_override_precedence() {
        let e = Parser::parse("(a | b) & c").unwrap();
        assert_eq!(
            e,
            Expr::and(
                Expr::or(Expr::Var("a".into()), Expr::Var("b".into())),
                Expr::Var("c".into())
            )
        );
    }

    #[test]
    fn unicode_operators() {
        let e = Parser::parse("a ∧ b ∨ c").unwrap();
        assert_eq!(
            e,
            Expr::or(
                Expr::and(Expr::Var("a".into()), Expr::Var("b".into())),
                Expr::Var("c".into())
            )
        );
        let e2 = Parser::parse("a → b").unwrap();
        assert!(matches!(e2, Expr::Implies(_, _)));
    }

    #[test]
    fn reports_unexpected_char() {
        let err = Parser::parse("a & @").unwrap_err();
        assert!(matches!(err, ParseError::UnexpectedChar { .. }));
    }

    #[test]
    fn reports_unclosed_paren() {
        let err = Parser::parse("(a & b").unwrap_err();
        assert!(matches!(err, ParseError::UnclosedParen { .. }));
    }

    #[test]
    fn reports_empty() {
        assert_eq!(Parser::parse("   ").unwrap_err(), ParseError::Empty);
    }
}
