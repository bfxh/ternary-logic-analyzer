//! 符号化等价判定：dual-rail 编码 + SAT。
//!
//! 每个三值量用两个布尔变量表示：`t`（取真）与 `x`（取未知），
//! 并加约束 `¬(t ∧ x)`；`(0,0)=False`、`(1,0)=True`、`(0,1)=Unknown`。
//!
//! 默认每个运算符按所选代数的真值表逐行加子句，K3/Ł3/B3 共用一套编码。
//! 若某些变量被声明为**已知二值**，它们及其所在的纯二值子图改走经典 CNF
//! （AND 3 条、OR 3 条、NOT 2 条、XOR 4 条），不再付 18 条三值子句的代价；
//! 在 0/1 上三套代数一致，因此快速路径与代数无关。
//!
//! 等价判定 = 断言两个根节点“不相等”，UNSAT 即等价；SAT 时从模型里取反例。

use std::collections::{HashMap, HashSet};

use varisat::{ExtendFormula, Lit, Solver, Var};

use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::value::Ternary;
use trit_eval::{Assignment, EquivalenceResult, assignment_index};

/// 符号化判定错误。
#[derive(Debug, thiserror::Error)]
pub enum SymbolicError {
    /// SAT 求解器报错。
    #[error("SAT 求解失败: {0}")]
    Solver(String),
}

/// 编码规模（变量数 / 子句数），用于验证二值快速路径的收益。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodingSize {
    /// SAT 变量数（含共享的 false 常量）。
    pub variables: usize,
    /// 子句数（含常量、门与等价断言）。
    pub clauses: usize,
}

type Rails = (Var, Var);

#[derive(Clone, Copy)]
struct Node {
    t: Var,
    x: Var,
    binary: bool,
}

struct Encoder {
    solver: Solver<'static>,
    next: usize,
    clauses: usize,
    alg: Algebra,
    false_var: Var,
    vars: HashMap<String, Node>,
    binary_vars: HashSet<String>,
}

impl Encoder {
    fn new(alg: Algebra, binary_vars: &HashSet<String>) -> Self {
        let mut solver = Solver::new();
        let false_var = Var::from_index(0);
        solver.add_clause(&[Self::lit(false_var, false)]);
        Self {
            solver,
            next: 1,
            clauses: 1,
            alg,
            false_var,
            vars: HashMap::new(),
            binary_vars: binary_vars.clone(),
        }
    }

    fn lit(var: Var, positive: bool) -> Lit {
        Lit::from_var(var, positive)
    }

    fn clause(&mut self, lits: &[Lit]) {
        self.solver.add_clause(lits);
        self.clauses += 1;
    }

    fn fresh_var(&mut self) -> Var {
        let var = Var::from_index(self.next);
        self.next += 1;
        var
    }

    /// 一对三值导轨，带 `¬(t ∧ x)`。
    fn ternary_pair(&mut self) -> Rails {
        let t = self.fresh_var();
        let x = self.fresh_var();
        self.clause(&[Self::lit(t, false), Self::lit(x, false)]);
        (t, x)
    }

    fn encode(&mut self, expr: &Expr) -> Node {
        let alg = self.alg;
        match expr {
            Expr::Const(value) => {
                let t = self.fresh_var();
                match value {
                    Ternary::False => {
                        self.clause(&[Self::lit(t, false)]);
                        Node {
                            t,
                            x: self.false_var,
                            binary: true,
                        }
                    }
                    Ternary::True => {
                        self.clause(&[Self::lit(t, true)]);
                        Node {
                            t,
                            x: self.false_var,
                            binary: true,
                        }
                    }
                    Ternary::Unknown => {
                        let x = self.fresh_var();
                        self.clause(&[Self::lit(t, false)]);
                        self.clause(&[Self::lit(x, true)]);
                        Node {
                            t,
                            x,
                            binary: false,
                        }
                    }
                }
            }
            Expr::Var(name) => {
                if let Some(node) = self.vars.get(name) {
                    return *node;
                }
                let node = if self.binary_vars.contains(name) {
                    Node {
                        t: self.fresh_var(),
                        x: self.false_var,
                        binary: true,
                    }
                } else {
                    let (t, x) = self.ternary_pair();
                    Node {
                        t,
                        x,
                        binary: false,
                    }
                };
                self.vars.insert(name.clone(), node);
                node
            }
            Expr::Not(a) => {
                let a = self.encode(a);
                if a.binary {
                    let y = self.fresh_var();
                    self.clause(&[Self::lit(a.t, true), Self::lit(y, true)]);
                    self.clause(&[Self::lit(a.t, false), Self::lit(y, false)]);
                    Node {
                        t: y,
                        x: self.false_var,
                        binary: true,
                    }
                } else {
                    let out = self.ternary_pair();
                    self.table(&[a], out, |v| alg.not(v[0]));
                    Node {
                        t: out.0,
                        x: out.1,
                        binary: false,
                    }
                }
            }
            Expr::And(a, b) => {
                let a = self.encode(a);
                let b = self.encode(b);
                if a.binary && b.binary {
                    let y = self.binary_and(a.t, b.t);
                    Node {
                        t: y,
                        x: self.false_var,
                        binary: true,
                    }
                } else {
                    let out = self.ternary_pair();
                    self.table(&[a, b], out, |v| alg.and(v[0], v[1]));
                    Node {
                        t: out.0,
                        x: out.1,
                        binary: false,
                    }
                }
            }
            Expr::Or(a, b) => {
                let a = self.encode(a);
                let b = self.encode(b);
                if a.binary && b.binary {
                    let y = self.binary_or(a.t, b.t);
                    Node {
                        t: y,
                        x: self.false_var,
                        binary: true,
                    }
                } else {
                    let out = self.ternary_pair();
                    self.table(&[a, b], out, |v| alg.or(v[0], v[1]));
                    Node {
                        t: out.0,
                        x: out.1,
                        binary: false,
                    }
                }
            }
            Expr::Xor(a, b) => {
                let a = self.encode(a);
                let b = self.encode(b);
                if a.binary && b.binary {
                    let y = self.binary_strong_add(a.t, b.t);
                    Node {
                        t: y,
                        x: self.false_var,
                        binary: true,
                    }
                } else {
                    let out = self.ternary_pair();
                    self.table(&[a, b], out, |v| alg.strong_add(v[0], v[1]));
                    Node {
                        t: out.0,
                        x: out.1,
                        binary: false,
                    }
                }
            }
            Expr::Implies(a, b) => {
                let a = self.encode(a);
                let b = self.encode(b);
                if a.binary && b.binary {
                    let y = self.binary_implies(a.t, b.t);
                    Node {
                        t: y,
                        x: self.false_var,
                        binary: true,
                    }
                } else {
                    let out = self.ternary_pair();
                    self.table(&[a, b], out, |v| alg.implies(v[0], v[1]));
                    Node {
                        t: out.0,
                        x: out.1,
                        binary: false,
                    }
                }
            }
            Expr::Equiv(a, b) => {
                let a = self.encode(a);
                let b = self.encode(b);
                if a.binary && b.binary {
                    let y = self.binary_equiv(a.t, b.t);
                    Node {
                        t: y,
                        x: self.false_var,
                        binary: true,
                    }
                } else {
                    let out = self.ternary_pair();
                    self.table(&[a, b], out, |v| alg.equiv(v[0], v[1]));
                    Node {
                        t: out.0,
                        x: out.1,
                        binary: false,
                    }
                }
            }
        }
    }

    /// 经典 CNF：`y = a ∧ b`。
    fn binary_and(&mut self, a: Var, b: Var) -> Var {
        let y = self.fresh_var();
        self.clause(&[Self::lit(a, false), Self::lit(b, false), Self::lit(y, true)]);
        self.clause(&[Self::lit(a, true), Self::lit(y, false)]);
        self.clause(&[Self::lit(b, true), Self::lit(y, false)]);
        y
    }

    /// 经典 CNF：`y = a ∨ b`。
    fn binary_or(&mut self, a: Var, b: Var) -> Var {
        let y = self.fresh_var();
        self.clause(&[Self::lit(a, true), Self::lit(b, true), Self::lit(y, false)]);
        self.clause(&[Self::lit(a, false), Self::lit(y, true)]);
        self.clause(&[Self::lit(b, false), Self::lit(y, true)]);
        y
    }

    /// 二值操作数上的 `⊕`（强加法）：K3/B3 是 `∧`，Ł3 是 `∨`——不是经典异或。
    fn binary_strong_add(&mut self, a: Var, b: Var) -> Var {
        match self.alg {
            Algebra::Lukasiewicz => self.binary_or(a, b),
            Algebra::Kleene | Algebra::Bochvar => self.binary_and(a, b),
        }
    }

    /// 经典 CNF：`y = a ⇒ b`。
    fn binary_implies(&mut self, a: Var, b: Var) -> Var {
        let y = self.fresh_var();
        self.clause(&[Self::lit(a, true), Self::lit(y, true)]);
        self.clause(&[Self::lit(b, false), Self::lit(y, true)]);
        self.clause(&[Self::lit(a, false), Self::lit(b, true), Self::lit(y, false)]);
        y
    }

    /// 经典 CNF：`y = (a ≡ b)`。
    fn binary_equiv(&mut self, a: Var, b: Var) -> Var {
        let y = self.fresh_var();
        self.clause(&[Self::lit(a, false), Self::lit(b, false), Self::lit(y, true)]);
        self.clause(&[Self::lit(a, true), Self::lit(b, true), Self::lit(y, true)]);
        self.clause(&[Self::lit(a, true), Self::lit(b, false), Self::lit(y, false)]);
        self.clause(&[Self::lit(a, false), Self::lit(b, true), Self::lit(y, false)]);
        y
    }

    /// 三值门：按真值表逐行加子句。
    ///
    /// 输出“恰好等于某值”是两根导轨的合取，必须拆成两条子句；
    /// 单条子句会放行其他取值。
    fn table(&mut self, ins: &[Node], out: Rails, f: impl Fn(&[Ternary]) -> Ternary) {
        let combos = 3usize.pow(ins.len() as u32);
        for idx in 0..combos {
            let mut rest = idx;
            let mut values = Vec::with_capacity(ins.len());
            let mut base = Vec::with_capacity(ins.len() * 2);
            for node in ins {
                let value = match rest % 3 {
                    0 => Ternary::False,
                    1 => Ternary::True,
                    _ => Ternary::Unknown,
                };
                rest /= 3;
                values.push(value);
                match value {
                    Ternary::False => {
                        base.push(Self::lit(node.t, true));
                        base.push(Self::lit(node.x, true));
                    }
                    Ternary::True => {
                        base.push(Self::lit(node.t, false));
                        base.push(Self::lit(node.x, true));
                    }
                    Ternary::Unknown => {
                        base.push(Self::lit(node.t, true));
                        base.push(Self::lit(node.x, false));
                    }
                }
            }
            let (t_pos, x_pos) = match f(&values) {
                Ternary::False => (false, false),
                Ternary::True => (true, false),
                Ternary::Unknown => (false, true),
            };
            let mut t_clause = base.clone();
            t_clause.push(Self::lit(out.0, t_pos));
            self.clause(&t_clause);
            let mut x_clause = base;
            x_clause.push(Self::lit(out.1, x_pos));
            self.clause(&x_clause);
        }
    }

    /// 断言 a 与 b 相等（用于最小反例的对偶检查）。
    fn assert_equal(&mut self, a: Node, b: Node) {
        let pairs = [
            (Ternary::False, Ternary::True),
            (Ternary::False, Ternary::Unknown),
            (Ternary::True, Ternary::False),
            (Ternary::True, Ternary::Unknown),
            (Ternary::Unknown, Ternary::False),
            (Ternary::Unknown, Ternary::True),
        ];
        for (va, vb) in pairs {
            let mut lits = neq_lits(a, va);
            lits.extend(neq_lits(b, vb));
            self.clause(&lits);
        }
    }

    fn assert_unequal(&mut self, a: Node, b: Node) {
        // 不相等 ⇔ 不能同时 False、不能同时 True、不能同时 Unknown。
        self.clause(&[
            Self::lit(a.t, true),
            Self::lit(a.x, true),
            Self::lit(b.t, true),
            Self::lit(b.x, true),
        ]);
        self.clause(&[
            Self::lit(a.t, false),
            Self::lit(a.x, true),
            Self::lit(b.t, false),
            Self::lit(b.x, true),
        ]);
        self.clause(&[
            Self::lit(a.t, true),
            Self::lit(a.x, false),
            Self::lit(b.t, true),
            Self::lit(b.x, false),
        ]);
    }
}

fn union_vars(a: &Expr, b: &Expr) -> Vec<String> {
    let mut vars = a.variables();
    for v in b.variables() {
        if !vars.contains(&v) {
            vars.push(v);
        }
    }
    vars
}

fn build(
    a: &Expr,
    b: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> (Encoder, Vec<String>) {
    let vars = union_vars(a, b);
    let mut enc = Encoder::new(alg, binary_vars);
    let ra = enc.encode(a);
    let rb = enc.encode(b);
    enc.assert_unequal(ra, rb);
    (enc, vars)
}

/// 用 SAT 判定等价；`binary_vars` 里的变量被固定为已知二值，其子图走经典 CNF。
pub fn equivalent_symbolic_with(
    a: &Expr,
    b: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> Result<EquivalenceResult, SymbolicError> {
    let (mut enc, vars) = build(a, b, alg, binary_vars);
    let sat = enc
        .solver
        .solve()
        .map_err(|e| SymbolicError::Solver(e.to_string()))?;
    if !sat {
        return Ok(EquivalenceResult {
            equivalent: true,
            counterexample: None,
            counterexample_assignment: None,
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
    let mut assignment = Assignment::new();
    for name in &vars {
        let node = enc.vars[name];
        let value = if trues.contains(&node.t) {
            Ternary::True
        } else if trues.contains(&node.x) {
            Ternary::Unknown
        } else {
            Ternary::False
        };
        assignment.set(name.clone(), value);
    }
    let index = assignment_index(&assignment, &vars);
    Ok(EquivalenceResult {
        equivalent: false,
        counterexample: index,
        counterexample_assignment: Some(assignment),
        variable_count: vars.len(),
    })
}

/// 用 SAT 判定等价（所有变量都按三值处理）。
pub fn equivalent_symbolic(
    a: &Expr,
    b: &Expr,
    alg: Algebra,
) -> Result<EquivalenceResult, SymbolicError> {
    equivalent_symbolic_with(a, b, alg, &HashSet::new())
}

/// 最小反例：只保留区分两边所必需的变量。
///
/// 返回 `None` 表示等价。返回的是**充分的**部分赋值：未出现的变量在各自域内
/// 任取，两边仍然不相等。最小化用对偶检查——删掉变量后若“两边相等”不可满足，
/// 才允许删除，因此不会得到“只保证存在某个补全”的空反例。
pub fn minimal_counterexample(
    a: &Expr,
    b: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> Result<Option<Assignment>, SymbolicError> {
    let (mut enc, vars) = build(a, b, alg, binary_vars);
    let sat = enc
        .solver
        .solve()
        .map_err(|e| SymbolicError::Solver(e.to_string()))?;
    if !sat {
        return Ok(None);
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
    let values = model_values(&enc, &vars, &trues);

    // 对偶编码：断言 a == b。固定保留变量后若仍可相等，说明该变量必须保留。
    let mut eq = Encoder::new(alg, binary_vars);
    let ea = eq.encode(a);
    let eb = eq.encode(b);
    eq.assert_equal(ea, eb);

    let mut keep = vec![true; vars.len()];
    for i in 0..vars.len() {
        keep[i] = false;
        let lits = assumptions(&eq, &vars, &values, &keep);
        eq.solver.assume(&lits);
        let sat = eq
            .solver
            .solve()
            .map_err(|e| SymbolicError::Solver(e.to_string()))?;
        if sat {
            keep[i] = true;
        }
    }

    let mut assignment = Assignment::new();
    for (i, name) in vars.iter().enumerate() {
        if keep[i] {
            assignment.set(name.clone(), values[i]);
        }
    }
    Ok(Some(assignment))
}

/// `node ≠ value` 的两个字面量（析取）。
fn neq_lits(node: Node, value: Ternary) -> Vec<Lit> {
    match value {
        Ternary::False => vec![Lit::from_var(node.t, true), Lit::from_var(node.x, true)],
        Ternary::True => vec![Lit::from_var(node.t, false), Lit::from_var(node.x, true)],
        Ternary::Unknown => vec![Lit::from_var(node.t, true), Lit::from_var(node.x, false)],
    }
}

fn model_values(enc: &Encoder, vars: &[String], trues: &HashSet<Var>) -> Vec<Ternary> {
    vars.iter()
        .map(|name| {
            let node = enc.vars[name];
            if trues.contains(&node.t) {
                Ternary::True
            } else if trues.contains(&node.x) {
                Ternary::Unknown
            } else {
                Ternary::False
            }
        })
        .collect()
}

fn assumptions(enc: &Encoder, vars: &[String], values: &[Ternary], keep: &[bool]) -> Vec<Lit> {
    let mut lits = Vec::new();
    for (i, name) in vars.iter().enumerate() {
        if !keep[i] {
            continue;
        }
        let node = enc.vars[name];
        match values[i] {
            Ternary::True => lits.push(Lit::from_var(node.t, true)),
            Ternary::Unknown => {
                lits.push(Lit::from_var(node.t, false));
                lits.push(Lit::from_var(node.x, true));
            }
            Ternary::False => {
                lits.push(Lit::from_var(node.t, false));
                lits.push(Lit::from_var(node.x, false));
            }
        }
    }
    lits
}

/// 是否存在合法赋值使表达式为 `True`；返回一个见证。
pub fn satisfiable(
    expr: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> Result<Option<Assignment>, SymbolicError> {
    let vars = expr.variables();
    let mut enc = Encoder::new(alg, binary_vars);
    let node = enc.encode(expr);
    enc.clause(&[Lit::from_var(node.t, true)]);
    enc.clause(&[Lit::from_var(node.x, false)]);
    let sat = enc
        .solver
        .solve()
        .map_err(|e| SymbolicError::Solver(e.to_string()))?;
    if !sat {
        return Ok(None);
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
    let mut assignment = Assignment::new();
    for name in &vars {
        let node = enc.vars[name];
        let value = if trues.contains(&node.t) {
            Ternary::True
        } else if trues.contains(&node.x) {
            Ternary::Unknown
        } else {
            Ternary::False
        };
        assignment.set(name.clone(), value);
    }
    Ok(Some(assignment))
}

/// 策略验证的元逻辑查询：前提**确定为 True**、断言**不为 True** 的合法赋值。
///
/// 三值的 `premise ⇒ assertion` 恒真并不等价于“只在前提为 True 的赋值上检查断言”：
/// 后者需要在导轨层直接约束 `premise = (t=1, x=0)` 与 `assertion.t = 0`。
pub fn premise_counterexample(
    premise: &Expr,
    assertion: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> Result<Option<Assignment>, SymbolicError> {
    let mut vars = premise.variables();
    for v in assertion.variables() {
        if !vars.contains(&v) {
            vars.push(v);
        }
    }
    let mut enc = Encoder::new(alg, binary_vars);
    let p = enc.encode(premise);
    let a = enc.encode(assertion);
    enc.clause(&[Lit::from_var(p.t, true)]);
    enc.clause(&[Lit::from_var(p.x, false)]);
    enc.clause(&[Lit::from_var(a.t, false)]);
    let sat = enc
        .solver
        .solve()
        .map_err(|e| SymbolicError::Solver(e.to_string()))?;
    if !sat {
        return Ok(None);
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
    let mut assignment = Assignment::new();
    for name in &vars {
        let node = enc.vars[name];
        let value = if trues.contains(&node.t) {
            Ternary::True
        } else if trues.contains(&node.x) {
            Ternary::Unknown
        } else {
            Ternary::False
        };
        assignment.set(name.clone(), value);
    }
    Ok(Some(assignment))
}

/// 只构建编码并返回规模，不求解；用于验证二值快速路径的收益。
pub fn encoding_size(
    a: &Expr,
    b: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> Result<EncodingSize, SymbolicError> {
    let (enc, _vars) = build(a, b, alg, binary_vars);
    Ok(EncodingSize {
        variables: enc.next,
        clauses: enc.clauses,
    })
}
