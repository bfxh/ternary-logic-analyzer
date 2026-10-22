use std::collections::HashSet;

use trit_bdd::equivalent_bdd;
use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::parser::Parser;
use trit_core::value::Ternary;
use trit_eval::eval::{decode_assignment, equivalent, eval};

fn parse(src: &str) -> Expr {
    Parser::parse(src).expect("测试表达式应可解析")
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

/// 只在“二值变量取 0/1”的子域上穷举。
fn binary_domain_exhaustive(a: &Expr, b: &Expr, alg: Algebra, binary: &HashSet<String>) -> bool {
    let vars = union_vars(a, b);
    for index in 0..3u64.pow(vars.len() as u32) {
        let asg = decode_assignment(index, &vars);
        if vars
            .iter()
            .any(|v| binary.contains(v) && !asg.get(v).expect("变量存在").is_binary())
        {
            continue;
        }
        if eval(a, &asg, alg) != eval(b, &asg, alg) {
            return false;
        }
    }
    true
}

#[test]
fn bdd_agrees_with_exhaustive() {
    let pairs = [
        ("a & b", "b & a"),
        ("a | b", "b | a"),
        ("a ^ b", "b ^ a"),
        ("a => b", "!a | b"),
        ("a ≡ b", "(a => b) & (b => a)"),
        ("a & (b | c)", "(a & b) | (a & c)"),
        ("a | (b & c)", "(a | b) & (a | c)"),
        ("a & 0", "0"),
        ("a & 1", "a"),
        ("a | X", "X"),
        ("a & X", "0"),
        ("!(a | b)", "!a & !b"),
        ("!(a & b)", "!a | !b"),
        ("a", "b"),
        ("a ^ a", "X"),
    ];
    for alg in Algebra::ALL {
        for (left, right) in pairs {
            let a = parse(left);
            let b = parse(right);
            let exhaustive = equivalent(&a, &b, alg);
            let bdd = equivalent_bdd(&a, &b, alg, &HashSet::new()).expect("BDD 不应报错");
            assert_eq!(
                exhaustive.equivalent, bdd.result.equivalent,
                "{alg:?}: {left} vs {right}"
            );
            assert_eq!(exhaustive.variable_count, bdd.result.variable_count);
            if let Some(idx) = bdd.result.counterexample {
                let vars = union_vars(&a, &b);
                let asg = decode_assignment(idx, &vars);
                assert_ne!(
                    eval(&a, &asg, alg),
                    eval(&b, &asg, alg),
                    "BDD 反例必须真的区分两边：{alg:?} {left} vs {right} idx={idx}"
                );
            }
        }
    }
}

#[test]
fn bdd_binary_domain_matches_oracle() {
    let binary: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
    let pairs = [
        ("a & b", "b & a"),
        ("a | b", "b | a"),
        ("a ^ b", "b ^ a"),
        ("a => b", "!a | b"),
        ("a ≡ b", "(a => b) & (b => a)"),
        ("a & c", "c & a"),
        ("a & 0", "0"),
        ("a & 1", "a"),
        ("a | X", "X"),
        ("a", "b"),
    ];
    for alg in Algebra::ALL {
        for (left, right) in pairs {
            let a = parse(left);
            let b = parse(right);
            let expected = binary_domain_exhaustive(&a, &b, alg, &binary);
            let bdd = equivalent_bdd(&a, &b, alg, &binary).expect("BDD 不应报错");
            assert_eq!(
                expected, bdd.result.equivalent,
                "{alg:?}: {left} vs {right}（二值域）"
            );
            if let Some(idx) = bdd.result.counterexample {
                let vars = union_vars(&a, &b);
                let asg = decode_assignment(idx, &vars);
                for name in ["a", "b"] {
                    if vars.iter().any(|v| v == name) {
                        assert!(asg.get(name).expect("变量存在").is_binary());
                    }
                }
            }
        }
    }
}

#[test]
fn bdd_handles_100_variables_with_small_diagrams() {
    let vars: Vec<String> = (0..100).map(|i| format!("a{i}")).collect();
    let chain = vars
        .iter()
        .map(|v| Expr::Var(v.clone()))
        .reduce(Expr::and)
        .unwrap();
    let same = equivalent_bdd(&chain, &chain, Algebra::Kleene, &HashSet::new()).expect("BDD");
    assert!(same.result.equivalent);
    assert!(
        same.nodes < 1000,
        "结构化公式的 BDD 应很小，nodes={}",
        same.nodes
    );

    let shorter = vars[..99]
        .iter()
        .map(|v| Expr::Var(v.clone()))
        .reduce(Expr::and)
        .unwrap();
    let diff = equivalent_bdd(&chain, &shorter, Algebra::Kleene, &HashSet::new()).expect("BDD");
    assert!(!diff.result.equivalent);
    assert!(
        diff.result.counterexample.is_none(),
        "100 变量的编号超出 u64，应为 None"
    );
    let asg = diff.result.counterexample_assignment.expect("应有完整反例");
    assert_ne!(
        eval(&chain, &asg, Algebra::Kleene),
        eval(&shorter, &asg, Algebra::Kleene)
    );
}

#[test]
fn bdd_binary_view_uses_fewer_variables() {
    // 100 个变量全部声明二值：BDD 只建 100 根导轨，而不是 200 根。
    let vars: Vec<String> = (0..100).map(|i| format!("a{i}")).collect();
    let chain = vars
        .iter()
        .map(|v| Expr::Var(v.clone()))
        .reduce(Expr::and)
        .unwrap();
    let binary: HashSet<String> = vars.iter().cloned().collect();
    let verdict = equivalent_bdd(&chain, &chain, Algebra::Kleene, &binary).expect("BDD");
    assert!(verdict.result.equivalent);
    assert!(verdict.nodes < 1000);
}

#[test]
fn bdd_counterexample_uses_ternary_values() {
    let a = parse("a & X");
    let b = parse("0");
    let verdict = equivalent_bdd(&a, &b, Algebra::Kleene, &HashSet::new()).expect("BDD");
    assert!(!verdict.result.equivalent);
    let asg = verdict.result.counterexample_assignment.expect("应有反例");
    // a=1 或 a=X 都能让 a&X = X ≠ 0；只要不是 0 即可。
    assert_ne!(asg.get("a"), Some(Ternary::False));
}

#[test]
fn binary_fast_path_uses_strong_add_not_xor() {
    for (alg, left, right) in [
        (Algebra::Kleene, "1 ^ 1", "1"),
        (Algebra::Kleene, "1 ^ 0", "0"),
        (Algebra::Lukasiewicz, "1 ^ 0", "1"),
        (Algebra::Bochvar, "1 ^ 0", "0"),
    ] {
        let verdict = equivalent_bdd(&parse(left), &parse(right), alg, &HashSet::new())
            .expect("BDD 不应报错");
        assert!(verdict.result.equivalent, "{alg:?}: {left} vs {right}");
    }

    let binary: HashSet<String> = ["a".to_string()].into_iter().collect();
    let a = parse("a ^ 1");
    let k3 = equivalent_bdd(&a, &parse("a"), Algebra::Kleene, &binary).expect("BDD");
    assert!(k3.result.equivalent, "K3: a ^ 1 ≡ a");
    let l3 = equivalent_bdd(&a, &parse("1"), Algebra::Lukasiewicz, &binary).expect("BDD");
    assert!(l3.result.equivalent, "Ł3: a ^ 1 ≡ 1");
}
