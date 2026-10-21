//! 表达式求值：把 [`Expr`] 在给定赋值或全部赋值上求值。

use std::collections::HashMap;

use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::bits::{Evaluator, TriBit, TruthTable};
use trit_core::value::Ternary;

/// 变量赋值：一组 `变量名 → 取值`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assignment {
    entries: Vec<(String, Ternary)>,
}

impl Assignment {
    /// 空赋值。
    pub fn new() -> Self {
        Self::default()
    }

    /// 绑定一个变量。
    pub fn set(&mut self, name: impl Into<String>, value: Ternary) -> &mut Self {
        let name = name.into();
        match self.entries.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = value,
            None => self.entries.push((name, value)),
        }
        self
    }

    /// 查询变量的取值；未绑定时返回 `None`。
    pub fn get(&self, name: &str) -> Option<Ternary> {
        self.entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| *v)
    }

    /// 已绑定的变量名。
    pub fn variables(&self) -> Vec<String> {
        self.entries.iter().map(|(n, _)| n.clone()).collect()
    }

    /// 赋值条目数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 单点求值：按赋值计算表达式值。
///
/// 未在赋值中出现的变量视为 `X`（未知）。
pub fn eval(expr: &Expr, assignment: &Assignment, alg: Algebra) -> Ternary {
    match expr {
        Expr::Const(v) => *v,
        Expr::Var(name) => assignment.get(name).unwrap_or(Ternary::Unknown),
        Expr::Not(a) => alg.not(eval(a, assignment, alg)),
        Expr::And(a, b) => alg.and(eval(a, assignment, alg), eval(b, assignment, alg)),
        Expr::Or(a, b) => alg.or(eval(a, assignment, alg), eval(b, assignment, alg)),
        Expr::Xor(a, b) => alg.strong_add(eval(a, assignment, alg), eval(b, assignment, alg)),
        Expr::Implies(a, b) => alg.implies(eval(a, assignment, alg), eval(b, assignment, alg)),
        Expr::Equiv(a, b) => alg.equiv(eval(a, assignment, alg), eval(b, assignment, alg)),
    }
}

/// 变量名 → 位平面下标的映射。
pub type VarMap = HashMap<String, usize>;

/// 位并行求值：把表达式编译成对位平面的操作。
pub fn eval_bitwise(expr: &Expr, planes: &[TriBit], vars: &VarMap, alg: Algebra) -> TriBit {
    match expr {
        Expr::Const(v) => TriBit::constant(*v),
        Expr::Var(name) => match vars.get(name) {
            Some(idx) => planes.get(*idx).copied().unwrap_or(TriBit::UNKNOWN),
            None => TriBit::UNKNOWN,
        },
        Expr::Not(a) => eval_bitwise(a, planes, vars, alg).not(),
        Expr::And(a, b) => {
            let (x, y) = (
                eval_bitwise(a, planes, vars, alg),
                eval_bitwise(b, planes, vars, alg),
            );
            x.and(y, alg)
        }
        Expr::Or(a, b) => {
            let (x, y) = (
                eval_bitwise(a, planes, vars, alg),
                eval_bitwise(b, planes, vars, alg),
            );
            x.or(y, alg)
        }
        Expr::Xor(a, b) => {
            let (x, y) = (
                eval_bitwise(a, planes, vars, alg),
                eval_bitwise(b, planes, vars, alg),
            );
            x.strong_add(y, alg)
        }
        Expr::Implies(a, b) => {
            let (x, y) = (
                eval_bitwise(a, planes, vars, alg),
                eval_bitwise(b, planes, vars, alg),
            );
            x.implies(y, alg)
        }
        Expr::Equiv(a, b) => {
            let (x, y) = (
                eval_bitwise(a, planes, vars, alg),
                eval_bitwise(b, planes, vars, alg),
            );
            x.equiv(y, alg)
        }
    }
}

/// 构造求值器并返回「变量名 → 下标」映射。
fn evaluator_for(vars: &[String], alg: Algebra) -> (Evaluator, VarMap) {
    let mut ev = Evaluator::new(alg);
    let mut map = VarMap::new();
    for name in vars {
        let idx = ev.declare(name.clone());
        map.insert(name.clone(), idx);
    }
    (ev, map)
}

/// 全真值表求值。
///
/// 变量顺序取自 [`Expr::variables`]（首次出现顺序）。
pub fn truth_table(expr: &Expr, alg: Algebra) -> TruthTable {
    let vars = expr.variables();
    let (ev, map) = evaluator_for(&vars, alg);
    ev.evaluate(|planes| eval_bitwise(expr, planes, &map, alg))
}

/// 两个表达式在给定语义下是否等价（真值表完全相同）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquivalenceResult {
    /// 是否等价。
    pub equivalent: bool,
    /// 第一个反例的赋值编号（`None` 表示等价）。
    pub counterexample: Option<u64>,
    /// 参与比较的变量数。
    pub variable_count: usize,
}

/// 判断两个表达式在给定语义下是否逻辑等价。
pub fn equivalent(a: &Expr, b: &Expr, alg: Algebra) -> EquivalenceResult {
    // 变量取并集，保证两边在同一组赋值上比较
    let mut vars = a.variables();
    for v in b.variables() {
        if !vars.contains(&v) {
            vars.push(v);
        }
    }
    let (ev, map) = evaluator_for(&vars, alg);
    let total = ev.assignment_count();
    let mut base = 0u64;
    while base < total {
        let planes = ev.chunk_planes(base);
        let va = eval_bitwise(a, &planes, &map, alg);
        let vb = eval_bitwise(b, &planes, &map, alg);
        let take = (total - base).min(64) as u32;
        for bit in 0..take {
            if va.get(bit) != vb.get(bit) {
                return EquivalenceResult {
                    equivalent: false,
                    counterexample: Some(base + bit as u64),
                    variable_count: vars.len(),
                };
            }
        }
        base += 64;
    }
    EquivalenceResult {
        equivalent: true,
        counterexample: None,
        variable_count: vars.len(),
    }
}

/// 把三进制赋值编号解码为各变量的取值。
pub fn decode_assignment(index: u64, vars: &[String]) -> Assignment {
    let mut asg = Assignment::new();
    let mut t = index;
    for name in vars {
        let digit = t % 3;
        t /= 3;
        let v = match digit {
            0 => Ternary::False,
            1 => Ternary::True,
            _ => Ternary::Unknown,
        };
        asg.set(name.clone(), v);
    }
    asg
}

#[cfg(test)]
mod tests {
    use super::*;
    use trit_core::parser::Parser;

    #[test]
    fn eval_single_assignment() {
        let e = Parser::parse("a & b").unwrap();
        let mut asg = Assignment::new();
        asg.set("a", Ternary::True).set("b", Ternary::False);
        assert_eq!(eval(&e, &asg, Algebra::Kleene), Ternary::False);
    }

    #[test]
    fn unbound_variable_is_unknown() {
        let e = Parser::parse("a").unwrap();
        assert_eq!(
            eval(&e, &Assignment::new(), Algebra::Kleene),
            Ternary::Unknown
        );
    }

    #[test]
    fn truth_table_covers_all_rows() {
        let e = Parser::parse("a | b").unwrap();
        let tt = truth_table(&e, Algebra::Kleene);
        assert_eq!(tt.variables.len(), 2);
        assert_eq!(tt.values.len(), 9);
    }

    #[test]
    fn truth_table_matches_scalar_evaluation() {
        let e = Parser::parse("(a & b) | (!a & c)").unwrap();
        let vars = e.variables();
        let tt = truth_table(&e, Algebra::Kleene);
        for (i, expected) in tt.values.iter().enumerate() {
            let asg = decode_assignment(i as u64, &vars);
            assert_eq!(eval(&e, &asg, Algebra::Kleene), *expected, "row {i}");
        }
    }

    #[test]
    fn equivalent_detects_equal_expressions() {
        let a = Parser::parse("a & b").unwrap();
        let b = Parser::parse("b & a").unwrap();
        let r = equivalent(&a, &b, Algebra::Kleene);
        assert!(r.equivalent);
        assert!(r.counterexample.is_none());
    }

    #[test]
    fn equivalent_finds_counterexample() {
        let a = Parser::parse("a & b").unwrap();
        let b = Parser::parse("a | b").unwrap();
        let r = equivalent(&a, &b, Algebra::Kleene);
        assert!(!r.equivalent);
        assert!(r.counterexample.is_some());
    }

    #[test]
    fn decode_assignment_round_trips() {
        let vars = vec!["a".to_string(), "b".to_string()];
        let asg = decode_assignment(0, &vars);
        assert_eq!(asg.get("a"), Some(Ternary::False));
        assert_eq!(asg.get("b"), Some(Ternary::False));
        let asg = decode_assignment(5, &vars);
        // 变量 0 取最低三进制位：5 = 2 + 1*3 → a=2(unknown), b=1(true)
        assert_eq!(asg.get("a"), Some(Ternary::Unknown));
        assert_eq!(asg.get("b"), Some(Ternary::True));
    }
}
