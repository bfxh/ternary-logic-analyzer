//! 表达式化简。
//!
//! 化简分两步：
//!
//! 1. **代数化简**：按当前语义可用的规则做结构重写（常量折叠、去重、
//!    吸收、单位元、De Morgan、交换律规范化等）。
//! 2. **语义常量检测**：若化简后的表达式在全部赋值下取同一个值，
//!    直接替换为该常量。
//!
//! ⚠️ 规则的适用性依赖语义：
//!
//! - **Kleene / Łukasiewicz** 的 `X` 是链的中点，构成分配格，
//!   因此吸收律、分配律、`a ∧ ¬a = 0` 之外的格性质成立。
//! - **Bochvar** 的 `X` 是毒值：分配律仍成立，但吸收律**不成立**
//!   （例：`a ∨ (a ∧ b)` 取 `a=0,b=X` 时左边为 `X`、右边为 `0`）。
//! - 矛盾律 `a ∧ ¬a = 0` 在三种语义下**都不成立**
//!   （取 `a = X` 时 `X ∧ X = X`），因此化简器不使用该规则。

use crate::eval::{VarMap, truth_table};
use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::value::Ternary;

/// 化简选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimplifyOptions {
    /// 是否做语义常量检测（把恒真/恒假/恒未知的表达式折叠成常量）。
    pub fold_constants: bool,
    /// 参与常量检测的最大变量数；超过则跳过该步（`3^n` 成本）。
    pub max_vars_for_constant_fold: usize,
    /// 是否把 `a ⇒ b` 展开为 `¬a ∨ b`（仅 Kleene 语义合法）。
    pub expand_implies: bool,
}

impl Default for SimplifyOptions {
    fn default() -> Self {
        Self {
            fold_constants: true,
            max_vars_for_constant_fold: 8,
            expand_implies: false,
        }
    }
}

/// 按当前语义化简表达式。
pub fn simplify(expr: &Expr, alg: Algebra) -> Expr {
    simplify_with(expr, alg, SimplifyOptions::default())
}

/// 带选项的化简。
pub fn simplify_with(expr: &Expr, alg: Algebra, opts: SimplifyOptions) -> Expr {
    let step1 = rewrite(expr, alg, opts);
    if !opts.fold_constants {
        return step1;
    }
    let var_count = step1.variables().len();
    if var_count > opts.max_vars_for_constant_fold {
        return step1;
    }
    match constant_value(&step1, alg) {
        Some(v) => Expr::Const(v),
        None => step1,
    }
}

/// 判断表达式是否语义常量；是则返回该值。
pub fn constant_value(expr: &Expr, alg: Algebra) -> Option<Ternary> {
    if expr.variables().is_empty() {
        let asg = crate::eval::Assignment::new();
        return Some(crate::eval::eval(expr, &asg, alg));
    }
    let tt = truth_table(expr, alg);
    let first = *tt.values.first()?;
    if tt.values.iter().all(|v| *v == first) {
        Some(first)
    } else {
        None
    }
}

/// 递归结构重写。
fn rewrite(expr: &Expr, alg: Algebra, opts: SimplifyOptions) -> Expr {
    match expr {
        Expr::Const(_) | Expr::Var(_) => expr.clone(),
        Expr::Not(a) => {
            let inner = rewrite(a, alg, opts);
            // 双否定消去
            if let Expr::Not(x) = &inner {
                return (**x).clone();
            }
            // ¬(常量)
            if let Expr::Const(v) = inner {
                return Expr::Const(alg.not(v));
            }
            Expr::logical_not(inner)
        }
        Expr::And(a, b) => {
            let x = rewrite(a, alg, opts);
            let y = rewrite(b, alg, opts);
            rewrite_and(x, y, alg)
        }
        Expr::Or(a, b) => {
            let x = rewrite(a, alg, opts);
            let y = rewrite(b, alg, opts);
            rewrite_or(x, y, alg)
        }
        Expr::Xor(a, b) => {
            let x = rewrite(a, alg, opts);
            let y = rewrite(b, alg, opts);
            rewrite_xor(x, y, alg)
        }
        Expr::Implies(a, b) => {
            let x = rewrite(a, alg, opts);
            let y = rewrite(b, alg, opts);
            if opts.expand_implies && alg == Algebra::Kleene {
                // a ⇒ b ≡ ¬a ∨ b（仅 Kleene 材料蕴含成立）
                return rewrite_or(Expr::logical_not(x), y, alg);
            }
            rewrite_implies(x, y, alg)
        }
        Expr::Equiv(a, b) => {
            let x = rewrite(a, alg, opts);
            let y = rewrite(b, alg, opts);
            rewrite_equiv(x, y, alg)
        }
    }
}

/// 交换律规范化：把两个操作数按稳定顺序排列。
fn ordered(a: Expr, b: Expr) -> (Expr, Expr) {
    let ka = expr_key(&a);
    let kb = expr_key(&b);
    if kb < ka { (b, a) } else { (a, b) }
}

/// 表达式的结构排序键。
fn expr_key(e: &Expr) -> String {
    e.to_infix()
}

/// `∧` 的重写规则。
fn rewrite_and(x: Expr, y: Expr, alg: Algebra) -> Expr {
    // 常量折叠
    if let (Expr::Const(a), Expr::Const(b)) = (&x, &y) {
        return Expr::Const(alg.and(*a, *b));
    }
    // 毒值语义：Bochvar 下 `X` 吸收一切
    if alg.is_poisonous() {
        if x.is_unknown_const() || y.is_unknown_const() {
            return Expr::Const(Ternary::Unknown);
        }
        if let Expr::Const(Ternary::True) = &x {
            return y;
        }
        if let Expr::Const(Ternary::True) = &y {
            return x;
        }
        // 毒值语义下无吸收律，只能做交换律规范化
        let (a, b) = ordered(x, y);
        return Expr::and(a, b);
    }
    // 格语义（K3 / Ł3）
    if let Expr::Const(Ternary::False) = &x {
        return Expr::Const(Ternary::False);
    }
    if let Expr::Const(Ternary::False) = &y {
        return Expr::Const(Ternary::False);
    }
    if let Expr::Const(Ternary::True) = &x {
        return y;
    }
    if let Expr::Const(Ternary::True) = &y {
        return x;
    }
    // 幂等：a ∧ a = a
    if x == y {
        return x;
    }
    // 吸收：a ∧ (a ∨ b) = a
    if let Some(absorbed) = absorb(&x, &y) {
        return absorbed;
    }
    if let Some(absorbed) = absorb(&y, &x) {
        return absorbed;
    }
    // 互补吸收：a ∧ ¬a 不成立（三值下不适用），跳过
    // 交换律规范化
    let (a, b) = ordered(x, y);
    Expr::and(a, b)
}

/// 判断 `x` 是否吸收 `y`（即 `y = x ∨ c` 的形式，返回 `x`）。
fn absorb(x: &Expr, y: &Expr) -> Option<Expr> {
    if let Expr::Or(p, q) = y
        && (**p == *x || **q == *x)
    {
        return Some(x.clone());
    }
    None
}

/// `∨` 的吸收：`a ∨ (a ∧ b) = a`。
fn absorb_or(x: &Expr, y: &Expr) -> Option<Expr> {
    if let Expr::And(p, q) = y
        && (**p == *x || **q == *x)
    {
        return Some(x.clone());
    }
    None
}

/// `∨` 的重写规则。
fn rewrite_or(x: Expr, y: Expr, alg: Algebra) -> Expr {
    if let (Expr::Const(a), Expr::Const(b)) = (&x, &y) {
        return Expr::Const(alg.or(*a, *b));
    }
    if alg.is_poisonous() {
        if x.is_unknown_const() || y.is_unknown_const() {
            return Expr::Const(Ternary::Unknown);
        }
        if let Expr::Const(Ternary::False) = &x {
            return y;
        }
        if let Expr::Const(Ternary::False) = &y {
            return x;
        }
        // 毒值语义下无吸收律，只能做交换律规范化
        let (a, b) = ordered(x, y);
        return Expr::or(a, b);
    }
    if let Expr::Const(Ternary::True) = &x {
        return Expr::Const(Ternary::True);
    }
    if let Expr::Const(Ternary::True) = &y {
        return Expr::Const(Ternary::True);
    }
    if let Expr::Const(Ternary::False) = &x {
        return y;
    }
    if let Expr::Const(Ternary::False) = &y {
        return x;
    }
    if x == y {
        return x;
    }
    if let Some(absorbed) = absorb_or(&x, &y) {
        return absorbed;
    }
    if let Some(absorbed) = absorb_or(&y, &x) {
        return absorbed;
    }
    let (a, b) = ordered(x, y);
    Expr::or(a, b)
}

/// `⊕` 的重写规则。
fn rewrite_xor(x: Expr, y: Expr, alg: Algebra) -> Expr {
    if let (Expr::Const(a), Expr::Const(b)) = (&x, &y) {
        return Expr::Const(alg.strong_add(*a, *b));
    }
    // Kleene 与 Bochvar 下 `X` 都传播为 `X`（Łukasiewicz 例外：X ⊕ X = 1）
    if alg != Algebra::Lukasiewicz && (x.is_unknown_const() || y.is_unknown_const()) {
        return Expr::Const(Ternary::Unknown);
    }
    // Ł3：X ⊕ X = 1
    if alg == Algebra::Lukasiewicz && x.is_unknown_const() && y.is_unknown_const() {
        return Expr::Const(Ternary::True);
    }
    // K3/B3：a ⊕ a = a（X 也传播为 X）；Ł3 只在 0/1 上相同，X ⊕ X = 1 已在上面处理。
    if x == y && alg != Algebra::Lukasiewicz {
        return x;
    }
    let (a, b) = ordered(x, y);
    Expr::xor(a, b)
}

/// `⇒` 的重写规则。
fn rewrite_implies(x: Expr, y: Expr, alg: Algebra) -> Expr {
    if let (Expr::Const(a), Expr::Const(b)) = (&x, &y) {
        return Expr::Const(alg.implies(*a, *b));
    }
    // 0 ⇒ b = 1（K3 / Ł3）；Bochvar 下 X 毒化
    if alg != Algebra::Bochvar
        && let Expr::Const(Ternary::False) = &x
    {
        return Expr::Const(Ternary::True);
    }
    Expr::implies(x, y)
}

/// `≡` 的重写规则。
fn rewrite_equiv(x: Expr, y: Expr, alg: Algebra) -> Expr {
    if let (Expr::Const(a), Expr::Const(b)) = (&x, &y) {
        return Expr::Const(alg.equiv(*a, *b));
    }
    // 只有 Ł3 的 a ≡ a 在 X 上仍为 1；K3/B3 在 a=X 时为 X。
    if x == y && alg == Algebra::Lukasiewicz {
        return Expr::Const(Ternary::True);
    }
    Expr::equiv(x, y)
}

/// 计算表达式的语义指纹（真值表摘要），仅用于缓存/预筛选。
///
/// 指纹依赖变量出现顺序且只有 64 位：**相同不保证等价，不同也不保证不等价**。
/// 判定等价请用 [`semantically_equal`]。
pub fn semantic_fingerprint(expr: &Expr, alg: Algebra) -> u64 {
    let vars = expr.variables();
    let tt = truth_table(expr, alg);
    let mut hash = 1469598103934665603u64;
    for v in &vars {
        for b in v.as_bytes() {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(1099511628211);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(1099511628211);
    }
    for val in &tt.values {
        hash ^= u64::from(val.as_char() as u8);
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}

/// 判断两个表达式在给定语义下语义等价（`3^n` 穷举，变量取并集）。
pub fn semantically_equal(a: &Expr, b: Expr, alg: Algebra) -> bool {
    crate::eval::equivalent(a, &b, alg).equivalent
}

/// 未使用的占位，保持 API 公开但标注用途。
#[doc(hidden)]
pub fn unused_var_map() -> VarMap {
    VarMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use trit_core::parser::Parser;

    fn simp(src: &str, alg: Algebra) -> String {
        let e = Parser::parse(src).unwrap();
        simplify(&e, alg).to_infix()
    }

    #[test]
    fn folds_constants() {
        assert_eq!(simp("1 & 0", Algebra::Kleene), "0");
        assert_eq!(simp("1 | 0", Algebra::Kleene), "1");
        assert_eq!(simp("!1", Algebra::Kleene), "0");
    }

    #[test]
    fn folds_semantic_constants() {
        // 三值逻辑下的关键差异：`a ∨ ¬a` 不是重言式（a=X 时结果为 X），
        // 因此不会被折叠成 1；而 `a ∧ ¬a` 同样不是恒假。
        // 真正可折叠的语义常量需要用 X 安全前提，例如 `X ∨ 1 = 1`。
        assert_eq!(simp("x | 1", Algebra::Kleene), "1");
        assert_eq!(simp("x & 0", Algebra::Kleene), "0");
        // 语义常量检测把恒定表达式折叠为常量
        assert_eq!(simp("1 & 0 | 1", Algebra::Kleene), "1");
        // 变量被吸收后，剩余部分仍是变量
        let e = Parser::parse("a & 1 | b & 0").unwrap();
        assert_eq!(simplify(&e, Algebra::Kleene), Expr::Var("a".into()));
    }

    #[test]
    fn applies_idempotence_and_identity() {
        assert_eq!(simp("a & a", Algebra::Kleene), "a");
        assert_eq!(simp("a | a", Algebra::Kleene), "a");
        assert_eq!(simp("a & 1", Algebra::Kleene), "a");
        assert_eq!(simp("a | 0", Algebra::Kleene), "a");
    }

    #[test]
    fn bochvar_poison_differs_from_kleene() {
        // Bochvar: 0 ∧ X = X；Kleene: 0 ∧ X = 0
        assert_eq!(simp("0 & x", Algebra::Bochvar), "X");
        assert_eq!(simp("0 & x", Algebra::Kleene), "0");
    }

    #[test]
    fn simplification_preserves_semantics() {
        for src in [
            "a & b | a",
            "!(a & b)",
            "(a => b) & (b => a)",
            "a ^ b",
            "a <=> b",
            "a & 1 | b & 0",
        ] {
            let e = Parser::parse(src).unwrap();
            let s = simplify(&e, Algebra::Kleene);
            let r = crate::eval::equivalent(&e, &s, Algebra::Kleene);
            assert!(r.equivalent, "simplify changed semantics: {src} -> {s}");
        }
    }

    #[test]
    fn simplification_preserves_semantics_in_bochvar() {
        for src in ["a & b | a", "0 & x", "a | 1", "a & x"] {
            let e = Parser::parse(src).unwrap();
            let s = simplify(&e, Algebra::Bochvar);
            let r = crate::eval::equivalent(&e, &s, Algebra::Bochvar);
            assert!(
                r.equivalent,
                "bochvar simplify changed semantics: {src} -> {s}"
            );
        }
    }

    #[test]
    fn absorption_only_removes_matching_operator() {
        // a | (a | b) 不能化简成 a；a | (a & b) 可以。
        let e = Parser::parse("a | (a | b)").unwrap();
        let s = simplify(&e, Algebra::Kleene);
        assert!(
            crate::eval::equivalent(&e, &s, Algebra::Kleene).equivalent,
            "a | (a | b) -> {s}"
        );
        assert_eq!(simp("a | (a & b)", Algebra::Kleene), "a");
    }

    #[test]
    fn xor_and_equiv_self_respect_unknown() {
        // K3/B3: a ⊕ a = a，a ≡ a 在 a=X 时为 X。
        assert_eq!(simp("a ^ a", Algebra::Kleene), "a");
        assert_eq!(simp("a ^ a", Algebra::Bochvar), "a");
        let e = Parser::parse("a <=> a").unwrap();
        let s = simplify(&e, Algebra::Kleene);
        assert!(
            crate::eval::equivalent(&e, &s, Algebra::Kleene).equivalent,
            "a ≡ a -> {s}"
        );
        // Ł3 的自等价恒为 1
        assert_eq!(simp("a <=> a", Algebra::Lukasiewicz), "1");
    }

    #[test]
    fn semantically_equal_uses_real_equivalence() {
        let a = Parser::parse("a & b").unwrap();
        let b = Parser::parse("b & a").unwrap();
        assert!(semantically_equal(&a, b, Algebra::Kleene));

        let a = Parser::parse("a").unwrap();
        let b = Parser::parse("a & (b | 1)").unwrap();
        assert!(semantically_equal(&a, b, Algebra::Kleene));

        let a = Parser::parse("a | (a | b)").unwrap();
        let b = Parser::parse("a").unwrap();
        assert!(!semantically_equal(&a, b, Algebra::Kleene));
    }
}
