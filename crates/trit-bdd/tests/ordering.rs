//! BDD 变量排序：顺序合法性、结论一致性与 best 选择。

use std::collections::HashSet;

use trit_bdd::{
    VarOrder, equivalent_bdd, equivalent_bdd_best, equivalent_bdd_with_order, order_variables,
    premise_counterexample_bdd, premise_counterexample_bdd_with_order, satisfiable_bdd,
    satisfiable_bdd_with_order,
};
use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::parser::Parser;

fn parse(src: &str) -> Expr {
    Parser::parse(src).expect("测试表达式应可解析")
}

#[test]
fn frequency_order_prefers_common_variables() {
    let a = parse("a & a & b");
    let b = parse("c");

    assert_eq!(
        order_variables(&a, &b, VarOrder::FrequencyDesc),
        vec!["a", "b", "c"]
    );
    assert_eq!(
        order_variables(&a, &b, VarOrder::FrequencyAsc),
        vec!["b", "c", "a"]
    );
    assert_eq!(
        order_variables(&a, &b, VarOrder::Reverse),
        vec!["c", "b", "a"]
    );
}

#[test]
fn ordered_bdd_keeps_verdict_and_fills_missing_variables() {
    let a = parse("(a & b) | c");
    let b = parse("c | (b & a)");
    let binary = HashSet::new();
    let base = equivalent_bdd(&a, &b, Algebra::Kleene, &binary).unwrap();

    for order in [
        VarOrder::Appearance,
        VarOrder::Reverse,
        VarOrder::FrequencyDesc,
        VarOrder::FrequencyAsc,
    ] {
        let vars = order_variables(&a, &b, order);
        let verdict = equivalent_bdd_with_order(&a, &b, Algebra::Kleene, &binary, &vars).unwrap();
        assert_eq!(
            verdict.result.equivalent,
            base.result.equivalent,
            "{} 改变了结论",
            order.as_str()
        );
        assert_eq!(verdict.result.variable_count, 3);
    }

    // 不完整顺序自动补齐
    let verdict =
        equivalent_bdd_with_order(&a, &b, Algebra::Kleene, &binary, &["c".to_string()]).unwrap();
    assert_eq!(verdict.result.variable_count, 3);
}

#[test]
fn best_order_is_never_worse_than_appearance() {
    let binary = HashSet::new();
    let cases = [
        ("(a & b) | (c & d)", "(a & b) | (d & c)"),
        ("a => (b => c)", "(a & b) => c"),
        ("(a | b) & (a | !b)", "a"),
    ];
    for (left, right) in cases {
        let a = parse(left);
        let b = parse(right);
        let base = equivalent_bdd(&a, &b, Algebra::Kleene, &binary).unwrap();
        let (best, order) = equivalent_bdd_best(&a, &b, Algebra::Kleene, &binary).unwrap();
        assert_eq!(
            best.result.equivalent, base.result.equivalent,
            "{left} vs {right}"
        );
        assert!(
            best.nodes <= base.nodes,
            "{left}: best {} 比 appearance {} 还大（{}）",
            best.nodes,
            base.nodes,
            order.as_str()
        );
    }
}

#[test]
fn ordered_satisfiable_and_premise_agree() {
    let expr = parse("(a & b) | c");
    let premise = parse("a");
    let binary = HashSet::new();
    let base_sat = satisfiable_bdd(&expr, Algebra::Kleene, &binary).unwrap();
    let base_check = premise_counterexample_bdd(&premise, &expr, Algebra::Kleene, &binary).unwrap();

    for order in [
        VarOrder::Appearance,
        VarOrder::Reverse,
        VarOrder::FrequencyDesc,
        VarOrder::FrequencyAsc,
    ] {
        let vars = order_variables(&expr, &expr, order);
        let sat = satisfiable_bdd_with_order(&expr, Algebra::Kleene, &binary, &vars).unwrap();
        assert_eq!(sat.is_some(), base_sat.is_some(), "{}", order.as_str());

        let vars = order_variables(&premise, &expr, order);
        let check =
            premise_counterexample_bdd_with_order(&premise, &expr, Algebra::Kleene, &binary, &vars)
                .unwrap();
        assert_eq!(
            check.counterexample.is_some(),
            base_check.counterexample.is_some(),
            "{}",
            order.as_str()
        );
    }
}
