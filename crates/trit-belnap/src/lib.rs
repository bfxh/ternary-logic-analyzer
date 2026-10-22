//! Belnap 四值逻辑（FOUR）：`T` / `F` / `B`（both）/ `N`（neither）。
//!
//! 这是 **opt-in** 语义：三值的 K3/Ł3/B3 不受影响，四值只在本 crate 里出现。
//!
//! 双轨表示：`T=(1,0)`、`F=(0,1)`、`B=(1,1)`、`N=(0,0)`。三值双导轨禁止 `(1,1)`，
//! 四值正是把这条约束放开——`B` 同时为真和为假，`N` 两者都不是。
//!
//! ```text
//! ∧ : t = t₁∧t₂，f = f₁∨f₂      // N∧B = F
//! ∨ : t = t₁∨t₂，f = f₁∧f₂      // N∨B = T
//! ¬ : 交换两轨
//! ⇒ : ¬a ∨ b
//! ```
//!
//! 注意 `⊕` 在 FOUR 里没有公认定义：这里取 `¬(a ⇔ b)`，`B ⊕ B = B`，不要按经典异或理解。

use std::collections::BTreeMap;

use trit_core::ast::Expr;
use trit_core::value::Ternary;

/// Belnap 四值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum B4 {
    /// 两者都不是（neither）。
    N,
    /// 假。
    F,
    /// 真。
    T,
    /// 同时为真和为假（both）。
    B,
}

impl B4 {
    /// 由双轨 `(真, 假)` 构造。
    pub const fn from_rails(t: bool, f: bool) -> Self {
        match (t, f) {
            (true, false) => Self::T,
            (false, true) => Self::F,
            (true, true) => Self::B,
            (false, false) => Self::N,
        }
    }

    /// 拆成双轨 `(真, 假)`。
    pub const fn to_rails(self) -> (bool, bool) {
        match self {
            Self::T => (true, false),
            Self::F => (false, true),
            Self::B => (true, true),
            Self::N => (false, false),
        }
    }

    /// 否定：交换两轨。
    pub const fn not(self) -> Self {
        let (t, f) = self.to_rails();
        Self::from_rails(f, t)
    }

    /// 合取：`t = t₁∧t₂`，`f = f₁∨f₂`。
    pub const fn and(self, other: Self) -> Self {
        let (t1, f1) = self.to_rails();
        let (t2, f2) = other.to_rails();
        Self::from_rails(t1 && t2, f1 || f2)
    }

    /// 析取：`t = t₁∨t₂`，`f = f₁∧f₂`。
    pub const fn or(self, other: Self) -> Self {
        let (t1, f1) = self.to_rails();
        let (t2, f2) = other.to_rails();
        Self::from_rails(t1 || t2, f1 && f2)
    }

    /// 蕴含：`¬a ∨ b`。
    pub const fn implies(self, other: Self) -> Self {
        self.not().or(other)
    }

    /// 等价：`(a ⇒ b) ∧ (b ⇒ a)`。
    pub const fn equiv(self, other: Self) -> Self {
        self.implies(other).and(other.implies(self))
    }

    /// 异或：`¬(a ⇔ b)`。FOUR 下不是经典异或。
    pub const fn xor(self, other: Self) -> Self {
        self.equiv(other).not()
    }

    /// 是否为“确定为真”的值（`T` 或 `B`）。
    pub const fn is_designated(self) -> bool {
        matches!(self, Self::T | Self::B)
    }

    /// 单字符表示：`N` / `F` / `T` / `B`。
    pub const fn as_char(self) -> char {
        match self {
            Self::N => 'N',
            Self::F => 'F',
            Self::T => 'T',
            Self::B => 'B',
        }
    }

    /// 解析单字符：`0/f`→F，`1/t`→T，`?/x`→N，`*/b`→B。
    pub fn parse_char(c: char) -> Option<Self> {
        match c {
            '0' | 'f' | 'F' => Some(Self::F),
            '1' | 't' | 'T' => Some(Self::T),
            '?' | 'x' | 'X' | 'n' | 'N' => Some(Self::N),
            '*' | 'b' | 'B' => Some(Self::B),
            _ => None,
        }
    }
}

impl std::fmt::Display for B4 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_char().to_string())
    }
}

impl From<Ternary> for B4 {
    /// 三值常量映射：`False→F`、`True→T`、`Unknown→N`。
    fn from(value: Ternary) -> Self {
        match value {
            Ternary::False => Self::F,
            Ternary::True => Self::T,
            Ternary::Unknown => Self::N,
        }
    }
}

/// 在给定赋值下求值；未出现的变量按 `N` 处理。
pub fn eval(expr: &Expr, assignment: &BTreeMap<String, B4>) -> B4 {
    match expr {
        Expr::Const(value) => B4::from(*value),
        Expr::Var(name) => assignment.get(name).copied().unwrap_or(B4::N),
        Expr::Not(inner) => eval(inner, assignment).not(),
        Expr::And(a, b) => eval(a, assignment).and(eval(b, assignment)),
        Expr::Or(a, b) => eval(a, assignment).or(eval(b, assignment)),
        Expr::Xor(a, b) => eval(a, assignment).xor(eval(b, assignment)),
        Expr::Implies(a, b) => eval(a, assignment).implies(eval(b, assignment)),
        Expr::Equiv(a, b) => eval(a, assignment).equiv(eval(b, assignment)),
    }
}

/// 一行真值表：按 `vars` 顺序的赋值与结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// 每个变量的取值。
    pub values: Vec<B4>,
    /// 表达式结果。
    pub value: B4,
}

/// 4^n 真值表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// 变量名（首次出现顺序）。
    pub vars: Vec<String>,
    /// 全部 `4^n` 行。
    pub rows: Vec<Row>,
}

/// 生成 4^n 真值表。`n` 较大时行数爆炸，调用方应先限制变量数。
pub fn table(expr: &Expr) -> Table {
    let vars = expr.variables();
    let count = 4usize.saturating_pow(vars.len() as u32);
    let mut rows = Vec::with_capacity(count.min(1 << 16));
    for combo in 0..count {
        let mut index = combo;
        let mut assignment = BTreeMap::new();
        let mut values = Vec::with_capacity(vars.len());
        for var in &vars {
            let value = match index % 4 {
                0 => B4::N,
                1 => B4::F,
                2 => B4::T,
                _ => B4::B,
            };
            index /= 4;
            values.push(value);
            assignment.insert(var.clone(), value);
        }
        rows.push(Row {
            values,
            value: eval(expr, &assignment),
        });
    }
    Table { vars, rows }
}

/// 在全部 `4^n` 赋值上判定两个表达式是否逐点相等。
///
/// `None` 表示等价；`Some(assignment)` 是一个反例。
pub fn equivalent(left: &Expr, right: &Expr) -> Option<BTreeMap<String, B4>> {
    let mut vars = left.variables();
    for var in right.variables() {
        if !vars.contains(&var) {
            vars.push(var);
        }
    }
    let count = 4usize.saturating_pow(vars.len() as u32);
    for combo in 0..count {
        let mut index = combo;
        let mut assignment = BTreeMap::new();
        for var in &vars {
            let value = match index % 4 {
                0 => B4::N,
                1 => B4::F,
                2 => B4::T,
                _ => B4::B,
            };
            index /= 4;
            assignment.insert(var.clone(), value);
        }
        if eval(left, &assignment) != eval(right, &assignment) {
            return Some(assignment);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use trit_core::parser::Parser;

    fn p(src: &str) -> Expr {
        Parser::parse(src).unwrap()
    }

    fn asg(pairs: &[(&str, B4)]) -> BTreeMap<String, B4> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn rails_roundtrip() {
        for v in [B4::N, B4::F, B4::T, B4::B] {
            let (t, f) = v.to_rails();
            assert_eq!(B4::from_rails(t, f), v);
        }
    }

    #[test]
    fn truth_tables_have_the_expected_conflicts() {
        assert_eq!(B4::N.and(B4::B), B4::F, "N∧B = F");
        assert_eq!(B4::N.or(B4::B), B4::T, "N∨B = T");
        assert_eq!(B4::B.not(), B4::B);
        assert_eq!(B4::B.implies(B4::F), B4::B);
        assert_eq!(B4::T.implies(B4::B), B4::B);
        assert_eq!(B4::T.and(B4::B), B4::B);
        assert_eq!(B4::F.or(B4::B), B4::B);
    }

    #[test]
    fn de_morgan_holds_on_all_sixteen_assignments() {
        let left = p("!(a & b)");
        let right = p("!a | !b");
        assert!(equivalent(&left, &right).is_none());
    }

    #[test]
    fn paraconsistent_pair_is_not_constant() {
        // a ∧ ¬a 在 a=B 时为 B、在 a=N 时为 N，所以与 0 不等价。
        let counterexample = equivalent(&p("a & !a"), &p("0")).expect("应当有反例");
        assert!(matches!(counterexample["a"], B4::N | B4::B));
        assert_eq!(B4::B.and(B4::B.not()), B4::B);
    }

    #[test]
    fn table_enumerates_four_to_the_n() {
        let t = table(&p("a & b"));
        assert_eq!(t.vars, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(t.rows.len(), 16);
    }

    #[test]
    fn eval_defaults_missing_variables_to_neither() {
        assert_eq!(eval(&p("a | b"), &asg(&[("a", B4::F)])), B4::N);
        assert_eq!(
            eval(&p("a & b"), &asg(&[("a", B4::N), ("b", B4::B)])),
            B4::F
        );
        assert_eq!(
            eval(&p("a | b"), &asg(&[("a", B4::N), ("b", B4::B)])),
            B4::T
        );
    }

    #[test]
    fn unknown_constant_maps_to_neither() {
        assert_eq!(B4::from(Ternary::Unknown), B4::N);
        assert_eq!(eval(&p("X | 1"), &BTreeMap::new()), B4::T);
    }
}
