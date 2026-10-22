//! 库级资源预算：AST 深度/节点数/枚举变量数。
//!
//! CLI 与各引擎入口共用同一套 `Limits`；默认值对交互式使用足够宽，
//! 调用方可以按场景收紧（例如 WASM 页面只允许 10 个变量）。

use crate::ast::Expr;

/// 资源预算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// AST 最大深度（根为 1）。
    pub max_depth: usize,
    /// AST 最大节点数。
    pub max_nodes: usize,
    /// 允许做 `3^n` 枚举的变量数上限。
    pub max_vars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 256,
            max_nodes: 100_000,
            max_vars: 12,
        }
    }
}

/// 预算超限。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LimitError {
    /// 深度超限。
    #[error("表达式深度超过 {limit} 层")]
    Depth {
        /// 上限。
        limit: usize,
    },
    /// 节点数超限。
    #[error("表达式节点数超过 {limit}")]
    Nodes {
        /// 上限。
        limit: usize,
    },
    /// 变量数超出枚举预算。
    #[error("表达式变量数超过 {limit}（3^n 枚举不可行）")]
    Vars {
        /// 上限。
        limit: usize,
    },
}

/// 迭代遍历 AST，检查深度与节点数（不递归，避免自身爆栈）。
pub fn check_expr(expr: &Expr, limits: Limits) -> Result<(), LimitError> {
    let mut stack: Vec<(&Expr, usize)> = vec![(expr, 1)];
    let mut nodes = 0usize;
    while let Some((node, depth)) = stack.pop() {
        if depth > limits.max_depth {
            return Err(LimitError::Depth {
                limit: limits.max_depth,
            });
        }
        nodes += 1;
        if nodes > limits.max_nodes {
            return Err(LimitError::Nodes {
                limit: limits.max_nodes,
            });
        }
        match node {
            Expr::Const(_) | Expr::Var(_) => {}
            Expr::Not(inner) => stack.push((inner, depth + 1)),
            Expr::And(a, b)
            | Expr::Or(a, b)
            | Expr::Xor(a, b)
            | Expr::Implies(a, b)
            | Expr::Equiv(a, b) => {
                stack.push((a, depth + 1));
                stack.push((b, depth + 1));
            }
        }
    }
    Ok(())
}

/// 检查变量数是否在 `3^n` 枚举预算内（应先调用 [`check_expr`] 保证深度安全）。
pub fn check_vars(expr: &Expr, limits: Limits) -> Result<(), LimitError> {
    let count = expr.variables().len();
    if count > limits.max_vars {
        return Err(LimitError::Vars {
            limit: limits.max_vars,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    #[test]
    fn default_limits_accept_ordinary_expressions() {
        let e = Parser::parse("(a & b) | (!c => d)").unwrap();
        assert!(check_expr(&e, Limits::default()).is_ok());
    }

    #[test]
    fn depth_limit_is_enforced() {
        let e = Parser::parse("!!!!a").unwrap();
        let tight = Limits {
            max_depth: 3,
            ..Limits::default()
        };
        assert!(matches!(
            check_expr(&e, tight),
            Err(LimitError::Depth { .. })
        ));
    }

    #[test]
    fn node_limit_is_enforced() {
        let e = Parser::parse("a & b & c & d").unwrap();
        let tight = Limits {
            max_nodes: 4,
            ..Limits::default()
        };
        assert!(matches!(
            check_expr(&e, tight),
            Err(LimitError::Nodes { .. })
        ));
    }

    #[test]
    fn var_limit_is_enforced() {
        let e = Parser::parse("a & b & c").unwrap();
        let tight = Limits {
            max_vars: 2,
            ..Limits::default()
        };
        assert!(matches!(
            check_vars(&e, tight),
            Err(LimitError::Vars { .. })
        ));
    }
}
