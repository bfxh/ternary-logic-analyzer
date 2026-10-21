//! 符号化等价判定：dual-rail 编码 + SAT。
//!
//! 每个三值量用两个布尔变量表示：`t`（取真）与 `x`（取未知），
//! 并加约束 `¬(t ∧ x)`；`(0,0)=False`、`(1,0)=True`、`(0,1)=Unknown`。
//! 每个运算符按所选代数的真值表逐行加子句，因此 K3/Ł3/B3 共用同一套编码。
//!
//! 等价判定 = 断言两个根节点“不相等”，UNSAT 即等价；SAT 时从模型里取反例。
//! 这把 `equivalent()` 的 3^n 穷举换成 SAT，变量数不再受真值表规模限制。

use std::collections::{HashMap, HashSet};

use varisat::{ExtendFormula, Lit, Solver, Var};

use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::value::Ternary;
use trit_eval::EquivalenceResult;

/// 符号化判定错误。
#[derive(Debug, thiserror::Error)]
pub enum SymbolicError {
    /// SAT 求解器报错。
    #[error("SAT 求解失败: {0}")]
    Solver(String),
}

type Rails = (Var, Var);

struct Encoder {
    solver: Solver<'static>,
    next: usize,
    alg: Algebra,
    vars: HashMap<String, Rails>,
}

impl Encoder {
    fn new(alg: Algebra) -> Self {
        Self {
            solver: Solver::new(),
            next: 0,
            alg,
            vars: HashMap::new(),
        }
    }

    fn lit(var: Var, positive: bool) -> Lit {
        Lit::from_var(var, positive)
    }

    /// 分配一对导轨并加上 `¬(t ∧ x)`。
    fn fresh(&mut self) -> Rails {
        let t = Var::from_index(self.next);
        let x = Var::from_index(self.next + 1);
        self.next += 2;
        self.solver
            .add_clause(&[Self::lit(t, false), Self::lit(x, false)]);
        (t, x)
    }

    fn fix(&mut self, rails: Rails, value: Ternary) {
        let (t, x) = rails;
        let (t_pos, x_pos) = match value {
            Ternary::False => (false, false),
            Ternary::True => (true, false),
            Ternary::Unknown => (false, true),
        };
        self.solver.add_clause(&[Self::lit(t, t_pos)]);
        self.solver.add_clause(&[Self::lit(x, x_pos)]);
    }

    /// 按真值表逐行加子句：对每个输入组合，若输入匹配则该输出必须匹配。
    fn table(&mut self, ins: &[Rails], out: Rails, f: impl Fn(&[Ternary]) -> Ternary) {
        let combos = 3usize.pow(ins.len() as u32);
        for idx in 0..combos {
            let mut rest = idx;
            let mut vals = Vec::with_capacity(ins.len());
            let mut clause = Vec::with_capacity(ins.len() * 2 + 2);
            for &(t, x) in ins {
                let v = match rest % 3 {
                    0 => Ternary::False,
                    1 => Ternary::True,
                    _ => Ternary::Unknown,
                };
                rest /= 3;
                vals.push(v);
                match v {
                    Ternary::False => {
                        clause.push(Self::lit(t, true));
                        clause.push(Self::lit(x, true));
                    }
                    Ternary::True => {
                        clause.push(Self::lit(t, false));
                        clause.push(Self::lit(x, true));
                    }
                    Ternary::Unknown => {
                        clause.push(Self::lit(t, true));
                        clause.push(Self::lit(x, false));
                    }
                }
            }
            // 输出“恰好等于某值”是两根轨道的合取，必须拆成两条子句；
            // 单条子句只能禁止一个输出取值，会放行别的取值。
            let (t_pos, x_pos) = match f(&vals) {
                Ternary::False => (false, false),
                Ternary::True => (true, false),
                Ternary::Unknown => (false, true),
            };
            let mut t_clause = clause.clone();
            t_clause.push(Self::lit(out.0, t_pos));
            self.solver.add_clause(&t_clause);
            let mut x_clause = clause;
            x_clause.push(Self::lit(out.1, x_pos));
            self.solver.add_clause(&x_clause);
        }
    }

    fn encode(&mut self, expr: &Expr) -> Rails {
        let alg = self.alg;
        match expr {
            Expr::Const(v) => {
                let rails = self.fresh();
                self.fix(rails, *v);
                rails
            }
            Expr::Var(name) => {
                if let Some(rails) = self.vars.get(name) {
                    *rails
                } else {
                    let rails = self.fresh();
                    self.vars.insert(name.clone(), rails);
                    rails
                }
            }
            Expr::Not(a) => {
                let ra = self.encode(a);
                let out = self.fresh();
                self.table(&[ra], out, |v| alg.not(v[0]));
                out
            }
            Expr::And(a, b) => {
                let ra = self.encode(a);
                let rb = self.encode(b);
                let out = self.fresh();
                self.table(&[ra, rb], out, |v| alg.and(v[0], v[1]));
                out
            }
            Expr::Or(a, b) => {
                let ra = self.encode(a);
                let rb = self.encode(b);
                let out = self.fresh();
                self.table(&[ra, rb], out, |v| alg.or(v[0], v[1]));
                out
            }
            Expr::Xor(a, b) => {
                let ra = self.encode(a);
                let rb = self.encode(b);
                let out = self.fresh();
                self.table(&[ra, rb], out, |v| alg.strong_add(v[0], v[1]));
                out
            }
            Expr::Implies(a, b) => {
                let ra = self.encode(a);
                let rb = self.encode(b);
                let out = self.fresh();
                self.table(&[ra, rb], out, |v| alg.implies(v[0], v[1]));
                out
            }
            Expr::Equiv(a, b) => {
                let ra = self.encode(a);
                let rb = self.encode(b);
                let out = self.fresh();
                self.table(&[ra, rb], out, |v| alg.equiv(v[0], v[1]));
                out
            }
        }
    }
}

/// 用 SAT 判定两个表达式在给定语义下是否等价。
///
/// 反例编号与 [`crate::eval::decode_assignment`] 的约定一致
/// （第 i 个三进制位对应变量 i：0=False、1=True、2=Unknown）。
pub fn equivalent_symbolic(
    a: &Expr,
    b: &Expr,
    alg: Algebra,
) -> Result<EquivalenceResult, SymbolicError> {
    let mut vars = a.variables();
    for v in b.variables() {
        if !vars.contains(&v) {
            vars.push(v);
        }
    }
    let mut enc = Encoder::new(alg);
    for name in &vars {
        let rails = enc.fresh();
        enc.vars.insert(name.clone(), rails);
    }
    let ra = enc.encode(a);
    let rb = enc.encode(b);

    // 两个三值量不相等 ⇔ 不能同时是 F、不能同时是 T、不能同时是 X。
    let l = |v: Var, p: bool| Encoder::lit(v, p);
    enc.solver
        .add_clause(&[l(ra.0, true), l(ra.1, true), l(rb.0, true), l(rb.1, true)]);
    enc.solver
        .add_clause(&[l(ra.0, false), l(ra.1, true), l(rb.0, false), l(rb.1, true)]);
    enc.solver
        .add_clause(&[l(ra.0, true), l(ra.1, false), l(rb.0, true), l(rb.1, false)]);

    let sat = enc
        .solver
        .solve()
        .map_err(|e| SymbolicError::Solver(e.to_string()))?;
    if !sat {
        return Ok(EquivalenceResult {
            equivalent: true,
            counterexample: None,
            variable_count: vars.len(),
        });
    }

    let model = enc
        .solver
        .model()
        .ok_or_else(|| SymbolicError::Solver("SAT 但没有模型".to_string()))?;
    let trues: HashSet<Var> = model
        .iter()
        .filter(|lit| lit.is_positive())
        .map(|lit| lit.var())
        .collect();
    let mut index = 0u64;
    let mut pow = 1u64;
    for name in &vars {
        let (t, x) = enc.vars[name];
        let digit = if trues.contains(&t) {
            1u64
        } else if trues.contains(&x) {
            2u64
        } else {
            0u64
        };
        index += digit * pow;
        pow *= 3;
    }
    Ok(EquivalenceResult {
        equivalent: false,
        counterexample: Some(index),
        variable_count: vars.len(),
    })
}
