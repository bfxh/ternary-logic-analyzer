//! 属性测试：符号化 SAT 判定必须与 3^n 穷举逐例一致。
//!
//! `trit-sat` 一直声明「穷举是 oracle」，但此前只有固定样例；这里用 proptest
//! 随机生成公式与语义，逐个比较等价结论，并复核 SAT 给出的反例/见证。

use std::collections::HashSet;

use proptest::prelude::*;

use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::value::Ternary;
use trit_eval::eval::{decode_assignment, equivalent, eval};
use trit_sat::symbolic::{equivalent_symbolic, satisfiable};

fn algebra_strategy() -> impl Strategy<Value = Algebra> {
    prop_oneof![
        Just(Algebra::Kleene),
        Just(Algebra::Lukasiewicz),
        Just(Algebra::Bochvar),
    ]
}

fn leaf() -> impl Strategy<Value = Expr> {
    prop_oneof![
        Just(Expr::Const(Ternary::False)),
        Just(Expr::Const(Ternary::True)),
        Just(Expr::Const(Ternary::Unknown)),
        Just(Expr::Var("a".to_string())),
        Just(Expr::Var("b".to_string())),
        Just(Expr::Var("c".to_string())),
    ]
}

fn formula() -> impl Strategy<Value = Expr> {
    leaf().prop_recursive(4, 40, 3, |inner| {
        prop_oneof![
            inner.clone().prop_map(Expr::logical_not),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| Expr::and(a, b)),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| Expr::or(a, b)),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| Expr::xor(a, b)),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| Expr::implies(a, b)),
            (inner.clone(), inner).prop_map(|(a, b)| Expr::equiv(a, b)),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    #[test]
    fn sat_matches_exhaustive(a in formula(), b in formula(), alg in algebra_strategy()) {
        let oracle = equivalent(&a, &b, alg);
        let sat = equivalent_symbolic(&a, &b, alg).expect("SAT 编码不应失败");
        prop_assert_eq!(sat.equivalent, oracle.equivalent);

        match sat.counterexample_assignment {
            Some(assignment) => {
                prop_assert_ne!(
                    eval(&a, &assignment, alg),
                    eval(&b, &assignment, alg),
                    "SAT 反例必须真的把两边区分开"
                );
            }
            None => prop_assert!(sat.equivalent, "不等价却没有反例"),
        }
    }

    #[test]
    fn satisfiable_witness_is_true(e in formula(), alg in algebra_strategy()) {
        let witness = satisfiable(&e, alg, &HashSet::new()).expect("SAT 编码不应失败");
        match witness {
            Some(assignment) => {
                prop_assert_eq!(eval(&e, &assignment, alg), Ternary::True);
            }
            None => {
                let vars = e.variables();
                let total = 3usize.saturating_pow(vars.len() as u32);
                for index in 0..total {
                    let assignment = decode_assignment(index as u64, &vars);
                    prop_assert_ne!(eval(&e, &assignment, alg), Ternary::True);
                }
            }
        }
    }
}
