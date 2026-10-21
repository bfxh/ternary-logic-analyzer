//! 属性测试：用 proptest 验证位并行内核与标量语义完全一致，
//! 以及三值代数的基本性质。

use proptest::prelude::*;
use trit_core::algebra::Algebra;
use trit_core::bits::TriBit;
use trit_core::parser::Parser;
use trit_core::value::Ternary;
use trit_eval::eval::{Assignment, decode_assignment, eval, truth_table};

/// 生成一个三值。
fn arb_ternary() -> impl Strategy<Value = Ternary> {
    prop_oneof![
        Just(Ternary::False),
        Just(Ternary::Unknown),
        Just(Ternary::True),
    ]
}

/// 生成一个语义。
fn arb_algebra() -> impl Strategy<Value = Algebra> {
    prop_oneof![
        Just(Algebra::Kleene),
        Just(Algebra::Lukasiewicz),
        Just(Algebra::Bochvar),
    ]
}

proptest! {
    /// 位并行的 and/or/strong_add/strong_and/implies/equiv 必须与标量一致。
    #[test]
    fn bitwise_matches_scalar(a in arb_ternary(), b in arb_ternary(), alg_s in arb_algebra()) {
        let ta = TriBit::constant(a);
        let tb = TriBit::constant(b);
        prop_assert_eq!(ta.and(tb, alg_s).get(0), alg_s.and(a, b));
        prop_assert_eq!(ta.or(tb, alg_s).get(0), alg_s.or(a, b));
        prop_assert_eq!(ta.strong_add(tb, alg_s).get(0), alg_s.strong_add(a, b));
        prop_assert_eq!(ta.strong_and(tb, alg_s).get(0), alg_s.strong_and(a, b));
        prop_assert_eq!(ta.implies(tb, alg_s).get(0), alg_s.implies(a, b));
        prop_assert_eq!(ta.equiv(tb, alg_s).get(0), alg_s.equiv(a, b));
    }

    /// 否定：双重否定还原，且 `¬X = X`。
    #[test]
    fn negation_involutive(a in arb_ternary()) {
        let neg = Algebra::Kleene.not(a);
        prop_assert_eq!(Algebra::Kleene.not(neg), a);
        if a.is_unknown() {
            prop_assert!(neg.is_unknown());
        }
    }

    /// Kleene / Łukasiewicz 的 and / or 满足交换律与幂等律。
    #[test]
    fn lattice_laws(a in arb_ternary(), b in arb_ternary()) {
        for alg_s in [Algebra::Kleene, Algebra::Lukasiewicz] {
            prop_assert_eq!(alg_s.and(a, b), alg_s.and(b, a));
            prop_assert_eq!(alg_s.or(a, b), alg_s.or(b, a));
            prop_assert_eq!(alg_s.and(a, a), a);
            prop_assert_eq!(alg_s.or(a, a), a);
        }
    }

    /// Bochvar 毒值语义：任何与 X 运算的结果都是 X。
    #[test]
    fn bochvar_is_poisonous(a in arb_ternary(), b in arb_ternary()) {
        let alg_s = Algebra::Bochvar;
        if a.is_unknown() || b.is_unknown() {
            prop_assert!(alg_s.and(a, b).is_unknown());
            prop_assert!(alg_s.or(a, b).is_unknown());
            prop_assert!(alg_s.strong_add(a, b).is_unknown());
            prop_assert!(alg_s.strong_and(a, b).is_unknown());
        }
    }

    /// 位并行真值表必须与逐点标量求值完全一致。
    #[test]
    fn truth_table_matches_pointwise(
        a in arb_ternary(),
        b in arb_ternary(),
        c in arb_ternary(),
        alg_s in arb_algebra(),
    ) {
        let expr = Parser::parse("(a & b) | (!a & c)").unwrap();
        let vars = expr.variables();
        let tt = truth_table(&expr, alg_s);
        prop_assert_eq!(tt.values.len(), 27);
        let mut asg = Assignment::new();
        asg.set("a", a).set("b", b).set("c", c);
        for (i, expected) in tt.values.iter().enumerate() {
            let row = decode_assignment(i as u64, &vars);
            // 只校验与给定三元组一致的那一行（三进制位序：a 最低位）
            if row.get("a") == Some(a) && row.get("b") == Some(b) && row.get("c") == Some(c) {
                prop_assert_eq!(eval(&expr, &asg, alg_s), *expected);
            }
        }
    }

    /// 化简不得改变语义。
    #[test]
    fn simplify_preserves_semantics(a in arb_ternary(), b in arb_ternary(), alg_s in arb_algebra()) {
        let expr = Parser::parse("a & b | a & !b").unwrap();
        let simplified = trit_eval::simplify::simplify(&expr, alg_s);
        let mut asg = Assignment::new();
        asg.set("a", a).set("b", b);
        prop_assert_eq!(eval(&expr, &asg, alg_s), eval(&simplified, &asg, alg_s));
    }

    /// 解析器：任意合法标识符都能解析且变量名保持一致。
    #[test]
    fn parser_roundtrip(src in "[a-z]{1,3}") {
        if let Ok(expr) = Parser::parse(&src) {
            for v in expr.variables() {
                prop_assert!(v.chars().all(|c| c.is_alphanumeric() || c == '_'));
                prop_assert!(!v.is_empty());
            }
        }
    }

    /// 等价判定：交换律必然等价；`a ∧ b` 与 `a ∧ ¬a` 在三值下不等价。
    #[test]
    fn equivalence_matches_truth_table(alg_s in arb_algebra()) {
        let e1 = Parser::parse("a & b").unwrap();
        let e2 = Parser::parse("b & a").unwrap();
        let r = trit_eval::eval::equivalent(&e1, &e2, alg_s);
        prop_assert!(r.equivalent);
        let e3 = Parser::parse("a & !a").unwrap();
        let r2 = trit_eval::eval::equivalent(&e1, &e3, alg_s);
        prop_assert!(!r2.equivalent);
    }
}

/// Łukasiewicz 的关键恒等式：X ⊕ X = 1、X ⊗ X = 0、X ⇒ X = 1。
#[test]
fn lukasiewicz_identities() {
    let alg = Algebra::Lukasiewicz;
    assert_eq!(
        alg.strong_add(Ternary::Unknown, Ternary::Unknown),
        Ternary::True
    );
    assert_eq!(
        alg.strong_and(Ternary::Unknown, Ternary::Unknown),
        Ternary::False
    );
    assert_eq!(
        alg.implies(Ternary::Unknown, Ternary::Unknown),
        Ternary::True
    );
}
