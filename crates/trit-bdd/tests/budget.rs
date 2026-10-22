//! BDD 节点预算：超限报错，放宽后正常。

use std::collections::HashSet;

use trit_bdd::{BddError, BddLimits, equivalent_bdd, equivalent_bdd_with_limits};
use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::parser::Parser;

fn parse(src: &str) -> Expr {
    Parser::parse(src).expect("测试表达式应可解析")
}

#[test]
fn budget_rejects_large_bdd() {
    let a = parse("(a & b) | (c & d)");
    let b = parse("(a & b) | (d & c)");
    let binary = HashSet::new();

    let err =
        equivalent_bdd_with_limits(&a, &b, Algebra::Kleene, &binary, BddLimits { max_nodes: 1 })
            .expect_err("1 个节点装不下");
    assert!(matches!(err, BddError::BudgetExceeded { limit: 1 }));

    let verdict =
        equivalent_bdd_with_limits(&a, &b, Algebra::Kleene, &binary, BddLimits::UNLIMITED).unwrap();
    assert!(verdict.result.equivalent);
    assert_eq!(
        verdict.result.equivalent,
        equivalent_bdd(&a, &b, Algebra::Kleene, &binary)
            .unwrap()
            .result
            .equivalent
    );
}
