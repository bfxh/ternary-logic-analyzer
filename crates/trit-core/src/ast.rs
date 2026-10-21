//! 表达式抽象语法树（AST）。
//!
//! 语法（优先级从低到高）：
//!
//! ```text
//! expr    := equiv
//! equiv   := implies ( ("≡" | "==" | "<=>") implies )*
//! implies := or ( ("=>" | "→") or )*            // 右结合
//! or      := xor ( ("∨" | "|" | "||") xor )*
//! xor     := and ( ("⊕" | "^") and )*
//! and     := unary ( ("∧" | "&" | "&&") unary )*
//! unary   := ("!" | "~" | "¬") unary | primary
//! primary := "(" expr ")" | ident | const
//! const   := "0" | "1" | "X" | "x" | "?" | "True" | "False"
//! ```
//!
//! `¬` 的优先级高于 `∧`，`∧` 高于 `∨`，`∨` 高于 `⊕` 之外的混合用法由
//! 上表固定；`⊕` 夹在 `∨` 与 `∧` 之间，表示强加法。

use serde::{Deserialize, Serialize};

/// 表达式节点。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Expr {
    /// 逻辑常量。
    Const(crate::value::Ternary),
    /// 变量引用。
    Var(String),
    /// 否定 `¬a`。
    Not(Box<Expr>),
    /// 与 `a ∧ b`。
    And(Box<Expr>, Box<Expr>),
    /// 或 `a ∨ b`。
    Or(Box<Expr>, Box<Expr>),
    /// 强加法 `a ⊕ b`。
    Xor(Box<Expr>, Box<Expr>),
    /// 蕴含 `a ⇒ b`（右结合）。
    Implies(Box<Expr>, Box<Expr>),
    /// 等价 `a ≡ b`。
    Equiv(Box<Expr>, Box<Expr>),
}

impl Expr {
    /// 构造与节点。
    pub fn and(a: Expr, b: Expr) -> Self {
        Self::And(Box::new(a), Box::new(b))
    }
    /// 构造或节点。
    pub fn or(a: Expr, b: Expr) -> Self {
        Self::Or(Box::new(a), Box::new(b))
    }
    /// 构造否定节点。
    ///
    /// 命名为 `logical_not` 而非 `not`，避免与 `std::ops::Not::not` 混淆。
    pub fn logical_not(a: Expr) -> Self {
        Self::Not(Box::new(a))
    }
    /// 构造强加法节点。
    pub fn xor(a: Expr, b: Expr) -> Self {
        Self::Xor(Box::new(a), Box::new(b))
    }
    /// 构造蕴含节点。
    pub fn implies(a: Expr, b: Expr) -> Self {
        Self::Implies(Box::new(a), Box::new(b))
    }
    /// 构造等价节点。
    pub fn equiv(a: Expr, b: Expr) -> Self {
        Self::Equiv(Box::new(a), Box::new(b))
    }

    /// 是否为常量 `Unknown`。
    pub const fn is_unknown_const(&self) -> bool {
        matches!(self, Self::Const(crate::value::Ternary::Unknown))
    }

    /// 收集表达式中出现的全部变量名（保持首次出现顺序）。
    pub fn variables(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        self.collect_vars(&mut out, &mut seen);
        out
    }

    fn collect_vars(&self, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>) {
        match self {
            Self::Const(_) => {}
            Self::Var(name) => {
                if seen.insert(name.clone()) {
                    out.push(name.clone());
                }
            }
            Self::Not(a) => a.collect_vars(out, seen),
            Self::And(a, b) | Self::Or(a, b) | Self::Xor(a, b) => {
                a.collect_vars(out, seen);
                b.collect_vars(out, seen);
            }
            Self::Implies(a, b) | Self::Equiv(a, b) => {
                a.collect_vars(out, seen);
                b.collect_vars(out, seen);
            }
        }
    }

    /// 节点总数（用于复杂度统计）。
    pub fn node_count(&self) -> usize {
        match self {
            Self::Const(_) | Self::Var(_) => 1,
            Self::Not(a) => 1 + a.node_count(),
            Self::And(a, b)
            | Self::Or(a, b)
            | Self::Xor(a, b)
            | Self::Implies(a, b)
            | Self::Equiv(a, b) => 1 + a.node_count() + b.node_count(),
        }
    }

    /// 树的最大深度。
    pub fn depth(&self) -> usize {
        match self {
            Self::Const(_) | Self::Var(_) => 1,
            Self::Not(a) => 1 + a.depth(),
            Self::And(a, b)
            | Self::Or(a, b)
            | Self::Xor(a, b)
            | Self::Implies(a, b)
            | Self::Equiv(a, b) => 1 + a.depth().max(b.depth()),
        }
    }

    /// 渲染为规范的中缀记法（带全角运算符），用于展示与化简输出。
    pub fn to_infix(&self) -> String {
        match self {
            Self::Const(v) => v.to_string(),
            Self::Var(name) => name.clone(),
            Self::Not(a) => format!("¬({})", a.to_infix()),
            Self::And(a, b) => format!("({} ∧ {})", a.to_infix(), b.to_infix()),
            Self::Or(a, b) => format!("({} ∨ {})", a.to_infix(), b.to_infix()),
            Self::Xor(a, b) => format!("({} ⊕ {})", a.to_infix(), b.to_infix()),
            Self::Implies(a, b) => format!("({} ⇒ {})", a.to_infix(), b.to_infix()),
            Self::Equiv(a, b) => format!("({} ≡ {})", a.to_infix(), b.to_infix()),
        }
    }
}

impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_infix())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Ternary;

    #[test]
    fn collects_variables_in_order() {
        let e = Expr::and(
            Expr::Var("a".into()),
            Expr::or(Expr::Var("b".into()), Expr::Var("a".into())),
        );
        assert_eq!(e.variables(), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn node_count_and_depth() {
        let e = Expr::and(Expr::Var("a".into()), Expr::Var("b".into()));
        assert_eq!(e.node_count(), 3);
        assert_eq!(e.depth(), 2);
    }

    #[test]
    fn infix_rendering() {
        let e = Expr::logical_not(Expr::Var("a".into()));
        assert_eq!(e.to_infix(), "¬(a)");
        let e2 = Expr::equiv(Expr::Const(Ternary::True), Expr::Var("x".into()));
        assert_eq!(e2.to_infix(), "(1 ≡ x)");
    }
}
