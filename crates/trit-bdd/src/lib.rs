//! 三值等价的 BDD（canonical）判定。
//!
//! 每个 trit 仍然是双导轨 `(t, x)`：`(0,0)=False`、`(1,0)=True`、`(0,1)=Unknown`。
//! 与 SAT 路径不同，这里把导轨直接建成 BDD：
//!
//! - 纯二值变量只建一根 `t` 导轨（`x ≡ false`），不付 unknown 的变量成本；
//! - 三值变量建 `t` / `x` 两根导轨，并在根上检查 `¬(t ∧ x)`；
//! - 门按代数真值表逐行构造（把每个输入组合的条件 OR 起来），K3/Ł3/B3 共用；
//! - 等价 = `valid ∧ ¬(a.t ↔ b.t ∧ a.x ↔ b.x)` 为 false（BDD 是 canonical 的）；
//! - 不等价时从差集 BDD 取一个满足赋值，还原成反例编号。
//!
//! BDD 的规模依赖变量顺序与函数结构；结构化公式通常很小，但某些函数会指数爆炸。
//! 默认变量顺序是表达式首次出现顺序。

use std::collections::{HashMap, HashSet};

use biodivine_lib_bdd::{Bdd, BddVariable, BddVariableSet};

use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::limits::{Limits, check_expr};
use trit_core::value::Ternary;
use trit_eval::{Assignment, EquivalenceResult, assignment_index};

/// BDD 判定错误。
#[derive(Debug, thiserror::Error)]
pub enum BddError {
    /// BDD 变量数超过 `u16` 上限。
    #[error("BDD 变量数 {0} 超过 u16 上限")]
    TooManyVariables(usize),
    /// BDD 说可满足但取不到赋值（不应发生）。
    #[error("BDD 缺少反例赋值")]
    MissingWitness,
    /// 被量化的变量不在表达式里。
    #[error("表达式里没有变量 {0}")]
    UnknownVariable(String),
    /// 表达式超出资源预算。
    #[error("表达式超出预算: {0}")]
    Limit(String),
    /// 单个 BDD 的节点数超出预算。
    #[error("BDD 节点数超过上限 {limit}")]
    BudgetExceeded {
        /// 上限。
        limit: usize,
    },
}

/// BDD 节点预算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BddLimits {
    /// 单个 BDD 允许的最大节点数（`Bdd::size`）。
    pub max_nodes: usize,
}

impl Default for BddLimits {
    fn default() -> Self {
        Self {
            max_nodes: 1_000_000,
        }
    }
}

impl BddLimits {
    /// 不设上限。
    pub const UNLIMITED: Self = Self {
        max_nodes: usize::MAX,
    };
}

/// BDD 判定结果，附带差集 BDD 的节点数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BddVerdict {
    /// 与其它引擎同形的结论。
    pub result: EquivalenceResult,
    /// 差集 BDD 的节点数（等价时为 0 或常量大小）。
    pub nodes: usize,
}

/// BDD 的静态变量排序策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarOrder {
    /// 首次出现顺序（默认）。
    Appearance,
    /// 首次出现顺序反转。
    Reverse,
    /// 出现次数降序，同频保持首次出现顺序。
    FrequencyDesc,
    /// 出现次数升序。
    FrequencyAsc,
}

impl VarOrder {
    /// 稳定短名。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::Reverse => "reverse",
            Self::FrequencyDesc => "frequency",
            Self::FrequencyAsc => "frequency-asc",
        }
    }

    /// 从 CLI 名解析。
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "appearance" => Some(Self::Appearance),
            "reverse" => Some(Self::Reverse),
            "frequency" | "frequency-desc" => Some(Self::FrequencyDesc),
            "frequency-asc" => Some(Self::FrequencyAsc),
            _ => None,
        }
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

fn merge_order(all: &[String], order: &[String]) -> Vec<String> {
    let mut vars: Vec<String> = Vec::with_capacity(all.len());
    for name in order {
        if all.contains(name) && !vars.contains(name) {
            vars.push(name.clone());
        }
    }
    for name in all {
        if !vars.contains(name) {
            vars.push(name.clone());
        }
    }
    vars
}

fn count_vars(expr: &Expr, counts: &mut HashMap<String, usize>) {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Var(name) => *counts.entry(name.clone()).or_default() += 1,
            Expr::Const(_) => {}
            Expr::Not(inner) => stack.push(inner),
            Expr::And(a, b)
            | Expr::Or(a, b)
            | Expr::Xor(a, b)
            | Expr::Implies(a, b)
            | Expr::Equiv(a, b) => {
                stack.push(a);
                stack.push(b);
            }
        }
    }
}

/// 按给定策略排列两个表达式的并集变量。
pub fn order_variables(a: &Expr, b: &Expr, order: VarOrder) -> Vec<String> {
    let vars = union_vars(a, b);
    match order {
        VarOrder::Appearance => vars,
        VarOrder::Reverse => vars.into_iter().rev().collect(),
        VarOrder::FrequencyDesc | VarOrder::FrequencyAsc => {
            let mut counts: HashMap<String, usize> = HashMap::new();
            count_vars(a, &mut counts);
            count_vars(b, &mut counts);
            let index: HashMap<String, usize> = vars
                .iter()
                .enumerate()
                .map(|(i, name)| (name.clone(), i))
                .collect();
            let mut ordered = vars;
            ordered.sort_by(|left, right| {
                let cl = counts.get(left.as_str()).copied().unwrap_or(0);
                let cr = counts.get(right.as_str()).copied().unwrap_or(0);
                let by_count = if order == VarOrder::FrequencyDesc {
                    cr.cmp(&cl)
                } else {
                    cl.cmp(&cr)
                };
                by_count.then_with(|| index[left.as_str()].cmp(&index[right.as_str()]))
            });
            ordered
        }
    }
}

#[derive(Clone)]
struct Node {
    t: Bdd,
    x: Bdd,
    binary: bool,
}

struct Encoder {
    vars: HashMap<String, Node>,
    varids: HashMap<String, (BddVariable, Option<BddVariable>)>,
    alg: Algebra,
    false_bdd: Bdd,
    true_bdd: Bdd,
    limits: BddLimits,
}

impl Encoder {
    fn new(
        alg: Algebra,
        vars: &[String],
        binary_vars: &HashSet<String>,
        limits: BddLimits,
    ) -> Result<Self, BddError> {
        let mut names: Vec<String> = Vec::with_capacity(vars.len() * 2);
        for var in vars {
            names.push(format!("{var}#t"));
            if !binary_vars.contains(var) {
                names.push(format!("{var}#x"));
            }
        }
        if names.len() > u16::MAX as usize {
            return Err(BddError::TooManyVariables(names.len()));
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let set = BddVariableSet::new(&refs);
        let false_bdd = set.mk_false();
        let true_bdd = set.mk_true();
        let mut varids = HashMap::with_capacity(vars.len());
        let mut nodes = HashMap::with_capacity(vars.len());
        for var in vars {
            let t_name = format!("{var}#t");
            let t_var = set.var_by_name(&t_name).ok_or(BddError::MissingWitness)?;
            let binary = binary_vars.contains(var);
            let (x_var, x_bdd) = if binary {
                (None, false_bdd.clone())
            } else {
                let x_name = format!("{var}#x");
                let x_var = set.var_by_name(&x_name).ok_or(BddError::MissingWitness)?;
                (Some(x_var), set.mk_var(x_var))
            };
            varids.insert(var.clone(), (t_var, x_var));
            nodes.insert(
                var.clone(),
                Node {
                    t: set.mk_var(t_var),
                    x: x_bdd,
                    binary,
                },
            );
        }
        Ok(Self {
            vars: nodes,
            varids,
            alg,
            false_bdd,
            true_bdd,
            limits,
        })
    }

    fn guard(&self, bdd: Bdd) -> Result<Bdd, BddError> {
        if bdd.size() > self.limits.max_nodes {
            Err(BddError::BudgetExceeded {
                limit: self.limits.max_nodes,
            })
        } else {
            Ok(bdd)
        }
    }

    fn encode(&self, expr: &Expr) -> Result<Node, BddError> {
        let alg = self.alg;
        Ok(match expr {
            Expr::Const(value) => match value {
                Ternary::False => Node {
                    t: self.false_bdd.clone(),
                    x: self.false_bdd.clone(),
                    binary: true,
                },
                Ternary::True => Node {
                    t: self.true_bdd.clone(),
                    x: self.false_bdd.clone(),
                    binary: true,
                },
                Ternary::Unknown => Node {
                    t: self.false_bdd.clone(),
                    x: self.true_bdd.clone(),
                    binary: false,
                },
            },
            Expr::Var(name) => self.vars[name].clone(),
            Expr::Not(a) => {
                let a = self.encode(a)?;
                if a.binary {
                    Node {
                        t: self.guard(a.t.not())?,
                        x: self.false_bdd.clone(),
                        binary: true,
                    }
                } else {
                    let (t, x) = self.table(&[a], |v| alg.not(v[0]))?;
                    Node {
                        t,
                        x,
                        binary: false,
                    }
                }
            }
            Expr::And(a, b) => self.binary_op(a, b, |x, y| x.and(y), |v| alg.and(v[0], v[1]))?,
            Expr::Or(a, b) => self.binary_op(a, b, |x, y| x.or(y), |v| alg.or(v[0], v[1]))?,
            Expr::Xor(a, b) => {
                // 二值操作数上 `⊕` 不是经典异或：K3/B3 是 `∧`，Ł3 是 `∨`。
                let ternary = |v: &[Ternary]| alg.strong_add(v[0], v[1]);
                match alg {
                    Algebra::Lukasiewicz => self.binary_op(a, b, |x, y| x.or(y), ternary)?,
                    Algebra::Kleene | Algebra::Bochvar => {
                        self.binary_op(a, b, |x, y| x.and(y), ternary)?
                    }
                }
            }
            Expr::Implies(a, b) => {
                self.binary_op(a, b, |x, y| x.not().or(y), |v| alg.implies(v[0], v[1]))?
            }
            Expr::Equiv(a, b) => {
                self.binary_op(a, b, |x, y| x.iff(y), |v| alg.equiv(v[0], v[1]))?
            }
        })
    }

    fn binary_op(
        &self,
        a: &Expr,
        b: &Expr,
        binary: impl Fn(&Bdd, &Bdd) -> Bdd,
        ternary: impl Fn(&[Ternary]) -> Ternary,
    ) -> Result<Node, BddError> {
        let a = self.encode(a)?;
        let b = self.encode(b)?;
        if a.binary && b.binary {
            Ok(Node {
                t: self.guard(binary(&a.t, &b.t))?,
                x: self.false_bdd.clone(),
                binary: true,
            })
        } else {
            let (t, x) = self.table(&[a, b], ternary)?;
            Ok(Node {
                t,
                x,
                binary: false,
            })
        }
    }

    /// 按真值表逐行构造两根输出导轨。
    fn table(
        &self,
        ins: &[Node],
        f: impl Fn(&[Ternary]) -> Ternary,
    ) -> Result<(Bdd, Bdd), BddError> {
        let mut t = self.false_bdd.clone();
        let mut x = self.false_bdd.clone();
        let combos = 3usize.pow(ins.len() as u32);
        for idx in 0..combos {
            let mut rest = idx;
            let mut values = Vec::with_capacity(ins.len());
            let mut condition = self.true_bdd.clone();
            for node in ins {
                let value = match rest % 3 {
                    0 => Ternary::False,
                    1 => Ternary::True,
                    _ => Ternary::Unknown,
                };
                rest /= 3;
                values.push(value);
                let bit = match value {
                    Ternary::False => node.t.not().and(&node.x.not()),
                    Ternary::True => node.t.and(&node.x.not()),
                    Ternary::Unknown => node.t.not().and(&node.x),
                };
                condition = self.guard(condition.and(&bit))?;
            }
            match f(&values) {
                Ternary::False => {}
                Ternary::True => t = self.guard(t.or(&condition))?,
                Ternary::Unknown => x = self.guard(x.or(&condition))?,
            }
        }
        Ok((t, x))
    }

    /// 所有三值变量的 `¬(t ∧ x)`。
    fn valid(&self) -> Result<Bdd, BddError> {
        let mut valid = self.true_bdd.clone();
        for node in self.vars.values() {
            if !node.binary {
                let ok = self.guard(node.t.not().or(&node.x.not()))?;
                valid = self.guard(valid.and(&ok))?;
            }
        }
        Ok(valid)
    }
}

fn assignment_from_valuation(enc: &Encoder, vars: &[String], bits: &[bool]) -> Assignment {
    let mut assignment = Assignment::new();
    for name in vars {
        let (t_var, x_var) = enc.varids[name];
        let t = bits[t_var.to_index()];
        let x = x_var.is_some_and(|v| bits[v.to_index()]);
        let value = if t {
            Ternary::True
        } else if x {
            Ternary::Unknown
        } else {
            Ternary::False
        };
        assignment.set(name.clone(), value);
    }
    assignment
}

/// 用 BDD 判定两个表达式在给定语义下是否等价。
pub fn equivalent_bdd(
    a: &Expr,
    b: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> Result<BddVerdict, BddError> {
    check_pair(a, b)?;
    let vars = union_vars(a, b);
    verdict_with_vars(a, b, alg, binary_vars, &vars, BddLimits::default())
}

/// 同 [`equivalent_bdd`]，但可指定 BDD 节点预算。
pub fn equivalent_bdd_with_limits(
    a: &Expr,
    b: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
    limits: BddLimits,
) -> Result<BddVerdict, BddError> {
    check_pair(a, b)?;
    let vars = union_vars(a, b);
    verdict_with_vars(a, b, alg, binary_vars, &vars, limits)
}

/// 用指定变量顺序判定；顺序里缺失的变量按首次出现顺序补在末尾。
pub fn equivalent_bdd_with_order(
    a: &Expr,
    b: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
    order: &[String],
) -> Result<BddVerdict, BddError> {
    check_pair(a, b)?;
    let union = union_vars(a, b);
    let mut vars: Vec<String> = Vec::with_capacity(union.len());
    for name in order {
        if union.contains(name) && !vars.contains(name) {
            vars.push(name.clone());
        }
    }
    for name in union {
        if !vars.contains(&name) {
            vars.push(name);
        }
    }
    verdict_with_vars(a, b, alg, binary_vars, &vars, BddLimits::default())
}

/// 试四种静态排序，取差集 BDD 节点数最少者；平局保留先出现者。
pub fn equivalent_bdd_best(
    a: &Expr,
    b: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> Result<(BddVerdict, VarOrder), BddError> {
    check_pair(a, b)?;
    let limits = BddLimits::default();
    let mut best: Option<(BddVerdict, VarOrder)> = None;
    for order in [
        VarOrder::Appearance,
        VarOrder::Reverse,
        VarOrder::FrequencyDesc,
        VarOrder::FrequencyAsc,
    ] {
        let vars = order_variables(a, b, order);
        let verdict = verdict_with_vars(a, b, alg, binary_vars, &vars, limits)?;
        if best
            .as_ref()
            .is_none_or(|(current, _)| verdict.nodes < current.nodes)
        {
            best = Some((verdict, order));
        }
    }
    Ok(best.expect("至少尝试一种排序"))
}

fn check_pair(a: &Expr, b: &Expr) -> Result<(), BddError> {
    for expr in [a, b] {
        check_expr(expr, Limits::default()).map_err(|e| BddError::Limit(e.to_string()))?;
    }
    Ok(())
}

fn verdict_with_vars(
    a: &Expr,
    b: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
    vars: &[String],
    limits: BddLimits,
) -> Result<BddVerdict, BddError> {
    let enc = Encoder::new(alg, vars, binary_vars, limits)?;
    let ra = enc.encode(a)?;
    let rb = enc.encode(b)?;
    let equal = enc.guard(ra.t.iff(&rb.t).and(&ra.x.iff(&rb.x)))?;
    let diff = enc.guard(enc.valid()?.and(&equal.not()))?;
    if diff.is_false() {
        return Ok(BddVerdict {
            result: EquivalenceResult {
                equivalent: true,
                counterexample: None,
                counterexample_assignment: None,
                variable_count: vars.len(),
            },
            nodes: diff.size(),
        });
    }
    let valuation = diff
        .sat_valuations()
        .next()
        .ok_or(BddError::MissingWitness)?;
    let assignment = assignment_from_valuation(&enc, vars, valuation.as_vector());
    let index = assignment_index(&assignment, vars);
    Ok(BddVerdict {
        result: EquivalenceResult {
            equivalent: false,
            counterexample: index,
            counterexample_assignment: Some(assignment),
            variable_count: vars.len(),
        },
        nodes: diff.size(),
    })
}

/// 是否存在合法赋值使表达式为 `True`；返回一个见证。
pub fn satisfiable_bdd(
    expr: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> Result<Option<Assignment>, BddError> {
    let vars = expr.variables();
    satisfiable_with_vars(expr, alg, binary_vars, &vars)
}

/// 指定变量顺序的可满足性判定；缺失变量按首次出现顺序补齐。
pub fn satisfiable_bdd_with_order(
    expr: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
    order: &[String],
) -> Result<Option<Assignment>, BddError> {
    let all = expr.variables();
    let vars = merge_order(&all, order);
    satisfiable_with_vars(expr, alg, binary_vars, &vars)
}

fn satisfiable_with_vars(
    expr: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
    vars: &[String],
) -> Result<Option<Assignment>, BddError> {
    check_expr(expr, Limits::default()).map_err(|e| BddError::Limit(e.to_string()))?;
    let enc = Encoder::new(alg, vars, binary_vars, BddLimits::default())?;
    let node = enc.encode(expr)?;
    // 必须限定在合法三值域内：非法导轨 (1,1) 可能被 table 解释成 False，再被 Not 变成 True。
    let sat = enc.guard(enc.valid()?.and(&node.t).and(&node.x.not()))?;
    if sat.is_false() {
        return Ok(None);
    }
    let valuation = sat
        .sat_valuations()
        .next()
        .ok_or(BddError::MissingWitness)?;
    Ok(Some(assignment_from_valuation(
        &enc,
        vars,
        valuation.as_vector(),
    )))
}

/// 前提过滤查询的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PremiseCheck {
    /// 反例；`None` 表示在前提为 True 的赋值上断言成立。
    pub counterexample: Option<Assignment>,
    /// 差集 BDD 的节点数。
    pub nodes: usize,
}

/// 策略验证的元逻辑查询：前提**确定为 True**、断言**不为 True** 的合法赋值。
pub fn premise_counterexample_bdd(
    premise: &Expr,
    assertion: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> Result<PremiseCheck, BddError> {
    let vars = union_vars(premise, assertion);
    premise_with_vars(premise, assertion, alg, binary_vars, &vars)
}

/// 指定变量顺序的前提过滤查询；缺失变量按首次出现顺序补齐。
pub fn premise_counterexample_bdd_with_order(
    premise: &Expr,
    assertion: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
    order: &[String],
) -> Result<PremiseCheck, BddError> {
    let all = union_vars(premise, assertion);
    let vars = merge_order(&all, order);
    premise_with_vars(premise, assertion, alg, binary_vars, &vars)
}

fn premise_with_vars(
    premise: &Expr,
    assertion: &Expr,
    alg: Algebra,
    binary_vars: &HashSet<String>,
    vars: &[String],
) -> Result<PremiseCheck, BddError> {
    for expr in [premise, assertion] {
        check_expr(expr, Limits::default()).map_err(|e| BddError::Limit(e.to_string()))?;
    }
    let enc = Encoder::new(alg, vars, binary_vars, BddLimits::default())?;
    let p = enc.encode(premise)?;
    let a = enc.encode(assertion)?;
    let bad = enc.guard(enc.valid()?.and(&p.t).and(&p.x.not()).and(&a.t.not()))?;
    let nodes = bad.size();
    let Some(valuation) = bad.sat_valuations().next() else {
        return Ok(PremiseCheck {
            counterexample: None,
            nodes,
        });
    };
    Ok(PremiseCheck {
        counterexample: Some(assignment_from_valuation(&enc, vars, valuation.as_vector())),
        nodes,
    })
}

/// 量化方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantifier {
    /// 存在量化：存在一组 trit 取值使表达式为真。
    Exists,
    /// 全称量化：对所有合法 trit 取值表达式都为真。
    Forall,
}

impl Quantifier {
    /// 解析 `exists` / `forall`。
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "exists" | "exist" | "some" => Some(Self::Exists),
            "forall" | "all" => Some(Self::Forall),
            _ => None,
        }
    }

    /// 稳定名称。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exists => "exists",
            Self::Forall => "forall",
        }
    }
}

/// 量化后的 BDD 与自由变量。
#[derive(Debug)]
pub struct Quantified {
    bdd: Bdd,
    valid_free: Bdd,
    free: Vec<String>,
    varids: HashMap<String, (BddVariable, Option<BddVariable>)>,
}

impl Quantified {
    /// 结果是否可满足。
    pub fn is_satisfiable(&self) -> bool {
        !self.bdd.is_false()
    }

    /// 结果是否恒真（对自由变量的所有合法取值）。
    pub fn is_valid(&self) -> bool {
        // 结果已经与合法域相交；对布尔全集恒真会把“还有三值自由变量”误判成不恒真。
        self.bdd.not().and(&self.valid_free).is_false()
    }

    /// 未被量化的变量（按首次出现顺序）。
    pub fn free_variables(&self) -> &[String] {
        &self.free
    }

    /// 量化后 BDD 的节点数。
    pub fn nodes(&self) -> usize {
        self.bdd.size()
    }

    /// 可满足时给一个自由变量的见证。
    pub fn witness(&self) -> Option<Assignment> {
        let valuation = self.bdd.sat_valuations().next()?;
        Some(self.decode(valuation.as_vector()))
    }

    /// 不全真时给一个自由变量的反例。
    pub fn counterexample(&self) -> Option<Assignment> {
        let negated = self.bdd.not().and(&self.valid_free);
        let valuation = negated.sat_valuations().next()?;
        Some(self.decode(valuation.as_vector()))
    }

    fn decode(&self, bits: &[bool]) -> Assignment {
        let mut assignment = Assignment::new();
        for name in &self.free {
            let (t_var, x_var) = self.varids[name];
            let t = bits[t_var.to_index()];
            let x = x_var.is_some_and(|v| bits[v.to_index()]);
            let value = if t {
                Ternary::True
            } else if x {
                Ternary::Unknown
            } else {
                Ternary::False
            };
            assignment.set(name.clone(), value);
        }
        assignment
    }
}

/// 对表达式做三值量化。
///
/// 量化的对象是 **trit**（三值变量），不是单根布尔导轨：
/// 三值变量只在 `{0,1,X}` 三个合法取值上量化；二值变量只在 `{0,1}` 上量化。
/// “真”指表达式**确定为 True**（不是 X）。自由变量仍要求合法（三值变量的两根导轨不同时为 1）。
pub fn quantify_bdd(
    expr: &Expr,
    quantified: &[String],
    kind: Quantifier,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> Result<Quantified, BddError> {
    let vars = expr.variables();
    quantify_with_vars(expr, quantified, kind, alg, binary_vars, &vars)
}

/// 指定变量顺序量化；顺序里缺失的变量按首次出现顺序补在末尾。
pub fn quantify_bdd_with_order(
    expr: &Expr,
    quantified: &[String],
    kind: Quantifier,
    alg: Algebra,
    binary_vars: &HashSet<String>,
    order: &[String],
) -> Result<Quantified, BddError> {
    let all = expr.variables();
    let mut vars: Vec<String> = Vec::with_capacity(all.len());
    for name in order {
        if all.contains(name) && !vars.contains(name) {
            vars.push(name.clone());
        }
    }
    for name in all {
        if !vars.contains(&name) {
            vars.push(name);
        }
    }
    quantify_with_vars(expr, quantified, kind, alg, binary_vars, &vars)
}

/// 试四种静态排序，取量化 BDD 节点数最少者；平局保留先出现者。
pub fn quantify_bdd_best(
    expr: &Expr,
    quantified: &[String],
    kind: Quantifier,
    alg: Algebra,
    binary_vars: &HashSet<String>,
) -> Result<(Quantified, VarOrder), BddError> {
    let mut best: Option<(Quantified, VarOrder)> = None;
    for order in [
        VarOrder::Appearance,
        VarOrder::Reverse,
        VarOrder::FrequencyDesc,
        VarOrder::FrequencyAsc,
    ] {
        let vars = order_variables(expr, expr, order);
        let quantified = quantify_with_vars(expr, quantified, kind, alg, binary_vars, &vars)?;
        if best
            .as_ref()
            .is_none_or(|(current, _)| quantified.nodes() < current.nodes())
        {
            best = Some((quantified, order));
        }
    }
    Ok(best.expect("至少尝试一种排序"))
}

fn quantify_with_vars(
    expr: &Expr,
    quantified: &[String],
    kind: Quantifier,
    alg: Algebra,
    binary_vars: &HashSet<String>,
    vars: &[String],
) -> Result<Quantified, BddError> {
    check_expr(expr, Limits::default()).map_err(|e| BddError::Limit(e.to_string()))?;
    for name in quantified {
        if !vars.iter().any(|v| v == name) {
            return Err(BddError::UnknownVariable(name.clone()));
        }
    }
    let free: Vec<String> = vars
        .iter()
        .filter(|v| !quantified.iter().any(|q| q == *v))
        .cloned()
        .collect();
    let enc = Encoder::new(alg, vars, binary_vars, BddLimits::default())?;
    let node = enc.encode(expr)?;
    let pred = node.t.and(&node.x.not());

    let mut q_rails = Vec::new();
    let mut valid_q = enc.true_bdd.clone();
    for name in quantified {
        let (t_var, x_var) = enc.varids[name];
        q_rails.push(t_var);
        if let Some(x_var) = x_var {
            q_rails.push(x_var);
            let n = &enc.vars[name];
            valid_q = enc.guard(valid_q.and(&n.t.not().or(&n.x.not())))?;
        }
    }
    let mut valid_free = enc.true_bdd.clone();
    for name in &free {
        let n = &enc.vars[name];
        if !n.binary {
            valid_free = enc.guard(valid_free.and(&n.t.not().or(&n.x.not())))?;
        }
    }

    let quantified_bdd = match kind {
        Quantifier::Exists => valid_q.and(&pred).exists(&q_rails),
        // ∀ valid_q ⇒ pred  ==  ¬∃ valid_q ∧ ¬pred
        Quantifier::Forall => valid_q.and(&pred.not()).exists(&q_rails).not(),
    };
    Ok(Quantified {
        bdd: enc.guard(quantified_bdd.and(&valid_free))?,
        valid_free,
        free,
        varids: enc.varids,
    })
}
