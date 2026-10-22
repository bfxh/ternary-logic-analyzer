//! 三值决策表的构建期验证。
//!
//! 策略文件是 JSON：变量分三值/二值，`definitions` 给出派生信号，
//! `assertions` 里每条表达式必须对所有合法赋值都为真。
//!
//! ```json
//! {
//!   "algebra": "kleene",
//!   "binary": ["infeasible", "contended"],
//!   "definitions": { "reject": "infeasible | (hopeless & contended)" },
//!   "assertions": [
//!     { "name": "硬约束", "expr": "infeasible => reject" },
//!     { "name": "空槽不拒", "expr": "(!contended & !infeasible) => !reject" }
//!   ]
//! }
//! ```
//!
//! 有 `assumptions` 时，断言只在**前提确定为 True** 的赋值上检查：
//! 查询是“前提为 True 且断言不为 True”是否可满足，而不是三值 `premise ⇒ assertion` 恒真
//! （后者会把 `a=X` 这类不满足前提的赋值也算进去）。

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::Deserialize;

use trit_bdd::{BddError, premise_counterexample_bdd, satisfiable_bdd};
use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::parser::Parser;
use trit_core::value::Ternary;
use trit_eval::Assignment;
use trit_sat::{SymbolicError, premise_counterexample, satisfiable};

/// 验证引擎。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// SAT + 最小反例。
    Sat,
    /// canonical BDD。
    Bdd,
}

impl Engine {
    /// 解析 `sat` / `bdd`。
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "sat" => Some(Self::Sat),
            "bdd" => Some(Self::Bdd),
            _ => None,
        }
    }

    /// 稳定名称。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sat => "sat",
            Self::Bdd => "bdd",
        }
    }
}

/// 策略文件。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// 语义，默认 `kleene`。
    #[serde(default = "default_algebra")]
    pub algebra: String,
    /// 已知二值的变量。
    #[serde(default)]
    pub binary: Vec<String>,
    /// 前提约束：必须可满足；断言只在满足前提的赋值上检查。
    #[serde(default)]
    pub assumptions: Vec<String>,
    /// 派生信号：名字 -> 表达式。
    #[serde(default)]
    pub definitions: BTreeMap<String, String>,
    /// 必须恒真的断言。
    pub assertions: Vec<Assertion>,
}

/// 一条断言。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    /// 断言名。
    pub name: String,
    /// 表达式。
    pub expr: String,
}

fn default_algebra() -> String {
    "kleene".to_string()
}

/// 单条断言的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionResult {
    /// 断言名。
    pub name: String,
    /// 是否恒真。
    pub holds: bool,
    /// 失败时的反例（SAT 为充分部分赋值，BDD 为完整赋值）。
    pub counterexample: Option<Assignment>,
    /// BDD 路径的差集节点数。
    pub nodes: Option<usize>,
}

/// 策略错误。
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// 读文件失败。
    #[error("读取策略失败: {0}")]
    Io(String),
    /// JSON 无效。
    #[error("策略 JSON 无效: {0}")]
    Json(String),
    /// 未知语义。
    #[error("未知语义: {0}（可选 kleene / lukasiewicz / bochvar）")]
    Algebra(String),
    /// 表达式解析失败。
    #[error("{what} 解析失败: {message}")]
    Parse {
        /// 出错的表达式名。
        what: String,
        /// 解析器消息。
        message: String,
    },
    /// 定义循环。
    #[error("定义循环: {0}")]
    Cycle(String),
    /// 前提不一致。
    #[error("前提不一致：没有合法赋值能让所有 assumptions 为 True")]
    InconsistentAssumptions,
    /// 策略没有任何断言。
    #[error("策略没有断言：拒绝作为门禁运行")]
    NoAssertions,
    /// SAT 引擎错误。
    #[error("SAT 错误: {0}")]
    Sat(String),
    /// BDD 引擎错误。
    #[error("BDD 错误: {0}")]
    Bdd(String),
}

impl From<serde_json::Error> for PolicyError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value.to_string())
    }
}

impl Policy {
    /// 从文件读取。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PolicyError> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| PolicyError::Io(format!("{}: {e}", path.as_ref().display())))?;
        Self::from_json(&text)
    }

    /// 从 JSON 文本解析。
    pub fn from_json(text: &str) -> Result<Self, PolicyError> {
        Ok(serde_json::from_str(text)?)
    }

    /// 逐条验证断言；任何一条失败都返回 `holds=false` 的结果，不提前中止。
    pub fn verify(&self, engine: Engine) -> Result<Vec<AssertionResult>, PolicyError> {
        if self.assertions.is_empty() {
            return Err(PolicyError::NoAssertions);
        }
        let alg = Algebra::parse(&self.algebra)
            .ok_or_else(|| PolicyError::Algebra(self.algebra.clone()))?;
        let binary: HashSet<String> = self.binary.iter().cloned().collect();
        let mut definitions = BTreeMap::new();
        for (name, src) in &self.definitions {
            definitions.insert(name.clone(), parse_expr(name, src)?);
        }
        let truth = Expr::Const(Ternary::True);
        let mut assumptions = Vec::with_capacity(self.assumptions.len());
        for (i, src) in self.assumptions.iter().enumerate() {
            let raw = parse_expr(&format!("assumption[{i}]"), src)?;
            assumptions.push(expand(&raw, &definitions, &mut Vec::new())?);
        }
        let premise = assumptions.into_iter().fold(truth.clone(), Expr::and);
        let satisfiable_premise = match engine {
            Engine::Sat => satisfiable(&premise, alg, &binary)
                .map_err(|e: SymbolicError| PolicyError::Sat(e.to_string()))?,
            Engine::Bdd => satisfiable_bdd(&premise, alg, &binary)
                .map_err(|e: BddError| PolicyError::Bdd(e.to_string()))?,
        };
        if satisfiable_premise.is_none() {
            return Err(PolicyError::InconsistentAssumptions);
        }

        let mut results = Vec::with_capacity(self.assertions.len());
        for assertion in &self.assertions {
            let raw = parse_expr(&assertion.name, &assertion.expr)?;
            let expr = expand(&raw, &definitions, &mut Vec::new())?;
            match engine {
                Engine::Sat => {
                    let witness = premise_counterexample(&premise, &expr, alg, &binary)
                        .map_err(|e: SymbolicError| PolicyError::Sat(e.to_string()))?;
                    results.push(AssertionResult {
                        name: assertion.name.clone(),
                        holds: witness.is_none(),
                        counterexample: witness,
                        nodes: None,
                    });
                }
                Engine::Bdd => {
                    let check = premise_counterexample_bdd(&premise, &expr, alg, &binary)
                        .map_err(|e: BddError| PolicyError::Bdd(e.to_string()))?;
                    results.push(AssertionResult {
                        name: assertion.name.clone(),
                        holds: check.counterexample.is_none(),
                        counterexample: check.counterexample,
                        nodes: Some(check.nodes),
                    });
                }
            }
        }
        Ok(results)
    }
}

fn parse_expr(what: &str, src: &str) -> Result<Expr, PolicyError> {
    Parser::parse(src).map_err(|e| PolicyError::Parse {
        what: what.to_string(),
        message: e.to_string(),
    })
}

fn expand(
    expr: &Expr,
    definitions: &BTreeMap<String, Expr>,
    stack: &mut Vec<String>,
) -> Result<Expr, PolicyError> {
    match expr {
        Expr::Const(_) => Ok(expr.clone()),
        Expr::Var(name) => {
            let Some(definition) = definitions.get(name) else {
                return Ok(expr.clone());
            };
            if stack.iter().any(|n| n == name) {
                let mut cycle = stack.clone();
                cycle.push(name.clone());
                return Err(PolicyError::Cycle(cycle.join(" -> ")));
            }
            stack.push(name.clone());
            let expanded = expand(definition, definitions, stack);
            stack.pop();
            expanded
        }
        Expr::Not(a) => Ok(Expr::logical_not(expand(a, definitions, stack)?)),
        Expr::And(a, b) => Ok(Expr::and(
            expand(a, definitions, stack)?,
            expand(b, definitions, stack)?,
        )),
        Expr::Or(a, b) => Ok(Expr::or(
            expand(a, definitions, stack)?,
            expand(b, definitions, stack)?,
        )),
        Expr::Xor(a, b) => Ok(Expr::xor(
            expand(a, definitions, stack)?,
            expand(b, definitions, stack)?,
        )),
        Expr::Implies(a, b) => Ok(Expr::implies(
            expand(a, definitions, stack)?,
            expand(b, definitions, stack)?,
        )),
        Expr::Equiv(a, b) => Ok(Expr::equiv(
            expand(a, definitions, stack)?,
            expand(b, definitions, stack)?,
        )),
    }
}
