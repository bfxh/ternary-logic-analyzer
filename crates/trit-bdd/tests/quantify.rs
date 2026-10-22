use std::collections::HashSet;

use trit_bdd::{
    BddError, Quantifier, VarOrder, order_variables, quantify_bdd, quantify_bdd_best,
    quantify_bdd_with_order,
};
use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::parser::Parser;
use trit_core::value::Ternary;

fn parse(src: &str) -> Expr {
    Parser::parse(src).expect("测试表达式应可解析")
}

#[test]
fn exists_over_one_variable_returns_free_witness() {
    let expr = parse("a & b");
    let q = quantify_bdd(
        &expr,
        &["a".to_string()],
        Quantifier::Exists,
        Algebra::Kleene,
        &HashSet::new(),
    )
    .expect("量化不应报错");
    assert!(q.is_satisfiable());
    assert_eq!(q.free_variables(), ["b".to_string()]);
    let witness = q.witness().expect("应有见证");
    assert_eq!(witness.get("b"), Some(Ternary::True));
    assert_eq!(witness.get("a"), None, "被量化变量不应出现在见证里");
}

#[test]
fn forall_over_one_variable_returns_free_counterexample() {
    let expr = parse("a & b");
    let q = quantify_bdd(
        &expr,
        &["a".to_string()],
        Quantifier::Forall,
        Algebra::Kleene,
        &HashSet::new(),
    )
    .expect("量化不应报错");
    assert!(!q.is_valid());
    let counterexample = q.counterexample().expect("应有反例");
    // 任何 b 都不能让 a&b 对 a 全称成立（a=0 即失败），所以只要求见证只含自由变量。
    assert_eq!(counterexample.variables(), ["b".to_string()]);
}

#[test]
fn forall_with_free_ternary_variable_is_judged_on_valid_domain() {
    // ∀a. a | (b | 1) 对所有合法 b（含 X）恒真。
    let expr = parse("a | (b | 1)");
    let q = quantify_bdd(
        &expr,
        &["a".to_string()],
        Quantifier::Forall,
        Algebra::Kleene,
        &HashSet::new(),
    )
    .expect("量化不应报错");
    assert!(q.is_valid(), "自由变量 b 下仍应恒真");
    assert!(q.counterexample().is_none());
}

#[test]
fn satisfiable_requires_a_legal_valuation() {
    // K3：!(a | !a) 在 a=0/1 为 0，在 a=X 为 X，没有让结果为 True 的合法赋值。
    let expr = parse("!(a | !a)");
    let witness =
        trit_bdd::satisfiable_bdd(&expr, Algebra::Kleene, &HashSet::new()).expect("求解不应报错");
    assert!(witness.is_none(), "非法导轨不能当作见证");
}

#[test]
fn forall_excluded_middle_depends_on_binary_domain() {
    let expr = parse("a | !a");
    let binary: HashSet<String> = ["a".to_string()].into_iter().collect();
    let binary_q = quantify_bdd(
        &expr,
        &["a".to_string()],
        Quantifier::Forall,
        Algebra::Kleene,
        &binary,
    )
    .expect("量化不应报错");
    assert!(binary_q.is_valid(), "二值域下排中律恒真");

    let ternary_q = quantify_bdd(
        &expr,
        &["a".to_string()],
        Quantifier::Forall,
        Algebra::Kleene,
        &HashSet::new(),
    )
    .expect("量化不应报错");
    assert!(!ternary_q.is_valid(), "三值域下 a=X 时 a|!a=X");
    let counterexample = ternary_q.counterexample().expect("应有反例");
    assert!(
        counterexample.is_empty(),
        "反例只含自由变量，而 a 已被全称量化掉：{counterexample:?}"
    );
}

#[test]
fn exists_of_contradiction_is_unsatisfiable() {
    let expr = parse("a & !a");
    let q = quantify_bdd(
        &expr,
        &["a".to_string()],
        Quantifier::Exists,
        Algebra::Kleene,
        &HashSet::new(),
    )
    .expect("量化不应报错");
    assert!(!q.is_satisfiable());
    assert!(q.witness().is_none());
}

#[test]
fn quantifying_all_variables_yields_a_boolean() {
    let expr = parse("a & b");
    let vars = ["a".to_string(), "b".to_string()];
    let exists = quantify_bdd(
        &expr,
        &vars,
        Quantifier::Exists,
        Algebra::Kleene,
        &HashSet::new(),
    )
    .expect("量化不应报错");
    assert!(exists.is_satisfiable());
    assert!(exists.free_variables().is_empty());
    assert!(exists.witness().is_some_and(|w| w.is_empty()));

    let forall = quantify_bdd(
        &expr,
        &vars,
        Quantifier::Forall,
        Algebra::Kleene,
        &HashSet::new(),
    )
    .expect("量化不应报错");
    assert!(!forall.is_valid());
    assert!(forall.counterexample().is_some_and(|c| c.is_empty()));
}

#[test]
fn unknown_variable_is_an_error() {
    let expr = parse("a & b");
    let err = quantify_bdd(
        &expr,
        &["c".to_string()],
        Quantifier::Exists,
        Algebra::Kleene,
        &HashSet::new(),
    )
    .unwrap_err();
    assert!(matches!(err, BddError::UnknownVariable(v) if v == "c"));
}

#[test]
fn ordering_does_not_change_quantified_result() {
    let expr = parse("(a & b) | (c & d)");
    let quantified = vec!["a".to_string(), "c".to_string()];
    let binary = HashSet::new();
    let base = quantify_bdd(
        &expr,
        &quantified,
        Quantifier::Forall,
        Algebra::Kleene,
        &binary,
    )
    .unwrap();

    for order in [
        VarOrder::Appearance,
        VarOrder::Reverse,
        VarOrder::FrequencyDesc,
        VarOrder::FrequencyAsc,
    ] {
        let vars = order_variables(&expr, &expr, order);
        let q = quantify_bdd_with_order(
            &expr,
            &quantified,
            Quantifier::Forall,
            Algebra::Kleene,
            &binary,
            &vars,
        )
        .unwrap();
        assert_eq!(
            q.is_valid(),
            base.is_valid(),
            "{} 改变了结论",
            order.as_str()
        );
        let mut got = q.free_variables().to_vec();
        let mut want = base.free_variables().to_vec();
        got.sort();
        want.sort();
        assert_eq!(got, want, "自由变量集合应一致（顺序可随排序变化）");
    }
}

#[test]
fn best_order_is_never_worse_than_appearance() {
    let expr = parse("(a & b) | (c & d)");
    let quantified = vec!["a".to_string()];
    let binary = HashSet::new();
    let base = quantify_bdd(
        &expr,
        &quantified,
        Quantifier::Exists,
        Algebra::Kleene,
        &binary,
    )
    .unwrap();
    let (best, order) = quantify_bdd_best(
        &expr,
        &quantified,
        Quantifier::Exists,
        Algebra::Kleene,
        &binary,
    )
    .unwrap();
    assert_eq!(best.is_satisfiable(), base.is_satisfiable());
    assert!(
        best.nodes() <= base.nodes(),
        "best {} 比 appearance {} 还大（{}）",
        best.nodes(),
        base.nodes(),
        order.as_str()
    );
}
