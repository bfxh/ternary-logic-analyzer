use std::collections::HashSet;
use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::parser::Parser;
use trit_core::value::Ternary;
use trit_eval::eval::{decode_assignment, equivalent, eval};

use trit_sat::symbolic::{
    encoding_size, equivalent_symbolic, equivalent_symbolic_with, minimal_counterexample,
};

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

#[test]
fn symbolic_agrees_with_exhaustive_on_small_formulas() {
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
    for alg in [Algebra::Kleene, Algebra::Lukasiewicz, Algebra::Bochvar] {
        for (left, right) in pairs {
            let a = parse(left);
            let b = parse(right);
            let exhaustive = equivalent(&a, &b, alg);
            let symbolic = equivalent_symbolic(&a, &b, alg).expect("SAT 不应报错");
            assert_eq!(
                exhaustive.equivalent, symbolic.equivalent,
                "{alg:?}: {left} vs {right}"
            );
            assert_eq!(exhaustive.variable_count, symbolic.variable_count);
            if let Some(idx) = symbolic.counterexample {
                let vars = union_vars(&a, &b);
                let asg = decode_assignment(idx, &vars);
                assert_ne!(
                    eval(&a, &asg, alg),
                    eval(&b, &asg, alg),
                    "SAT 反例必须真的区分两边：{alg:?} {left} vs {right} idx={idx}"
                );
            }
        }
    }
}

#[test]
fn symbolic_handles_thirty_variables() {
    let vars: Vec<String> = (0..30).map(|i| format!("a{i}")).collect();
    let chain = vars
        .iter()
        .map(|v| Expr::Var(v.clone()))
        .reduce(Expr::and)
        .unwrap();
    let same = equivalent_symbolic(&chain, &chain, Algebra::Kleene).expect("SAT 不应报错");
    assert!(same.equivalent);

    let shorter = vars[..29]
        .iter()
        .map(|v| Expr::Var(v.clone()))
        .reduce(Expr::and)
        .unwrap();
    let diff = equivalent_symbolic(&chain, &shorter, Algebra::Kleene).expect("SAT 不应报错");
    assert!(!diff.equivalent);
    let idx = diff.counterexample.expect("应有反例");
    let asg = decode_assignment(idx, &vars);
    assert_ne!(
        eval(&chain, &asg, Algebra::Kleene),
        eval(&shorter, &asg, Algebra::Kleene)
    );
}

/// 只在“二值变量取 0/1”的子域上穷举，作为条件性编码的 oracle。
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
fn binary_fast_path_agrees_on_the_binary_domain() {
    let binary: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
    let pairs = [
        ("a & b", "b & a"),
        ("a | b", "b | a"),
        ("a ^ b", "b ^ a"),
        ("a => b", "!a | b"),
        ("a ≡ b", "(a => b) & (b => a)"),
        ("a & c", "c & a"),
        ("a | c", "c | a"),
        ("a & 0", "0"),
        ("a & 1", "a"),
        ("a | X", "X"),
        ("a", "b"),
        ("a ^ a", "X"),
    ];
    for alg in Algebra::ALL {
        for (left, right) in pairs {
            let a = parse(left);
            let b = parse(right);
            let expected = binary_domain_exhaustive(&a, &b, alg, &binary);
            let fast = equivalent_symbolic_with(&a, &b, alg, &binary).expect("SAT 不应报错");
            assert_eq!(
                expected, fast.equivalent,
                "{alg:?}: {left} vs {right}（二值域）"
            );
            if let Some(idx) = fast.counterexample {
                let vars = union_vars(&a, &b);
                let asg = decode_assignment(idx, &vars);
                for name in ["a", "b"] {
                    if vars.iter().any(|v| v == name) {
                        assert!(
                            asg.get(name).expect("变量存在").is_binary(),
                            "二值变量不应取 Unknown：{alg:?} {left} vs {right}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn binary_fast_path_shrinks_the_encoding() {
    let vars: Vec<String> = (0..100).map(|i| format!("a{i}")).collect();
    let chain = vars
        .iter()
        .map(|v| Expr::Var(v.clone()))
        .reduce(Expr::and)
        .unwrap();
    let all: HashSet<String> = vars.iter().cloned().collect();
    let generic =
        encoding_size(&chain, &chain, Algebra::Kleene, &HashSet::new()).expect("编码不应报错");
    let fast = encoding_size(&chain, &chain, Algebra::Kleene, &all).expect("编码不应报错");
    assert!(
        fast.clauses * 2 < generic.clauses,
        "二值快速路径应至少减半：generic={generic:?} fast={fast:?}"
    );
    assert!(
        fast.variables < generic.variables,
        "二值快速路径应更少变量：generic={generic:?} fast={fast:?}"
    );
}

#[test]
fn minimal_counterexample_none_for_equivalent() {
    let a = parse("a & b");
    let b = parse("b & a");
    assert!(
        minimal_counterexample(&a, &b, Algebra::Kleene, &HashSet::new())
            .expect("SAT 不应报错")
            .is_none()
    );
}

#[test]
fn minimal_counterexample_drops_irrelevant_vars() {
    // b 二值时 b | !b = 1，左式只依赖 a；b 应被最小化掉。
    let a = parse("a & (b | !b)");
    let b = parse("0");
    let binary: HashSet<String> = ["b"].iter().map(|s| s.to_string()).collect();
    let witness = minimal_counterexample(&a, &b, Algebra::Kleene, &binary)
        .expect("SAT 不应报错")
        .expect("应不等价");
    assert_eq!(witness.get("a"), Some(Ternary::True), "a 必须保留为 1");
    assert_eq!(witness.get("b"), None, "b 应被最小化掉");
    assert_eq!(witness.len(), 1);
}

#[test]
fn minimal_counterexample_respects_binary_domain() {
    let a = parse("a & b");
    let b = parse("a");
    let binary: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
    let witness = minimal_counterexample(&a, &b, Algebra::Kleene, &binary)
        .expect("SAT 不应报错")
        .expect("应不等价");
    assert_eq!(witness.get("a"), Some(Ternary::True));
    assert_eq!(witness.get("b"), Some(Ternary::False));
    assert_eq!(witness.len(), 2);
}

#[test]
fn minimal_counterexample_is_sufficient() {
    // 反例固定 a=1；b 二值自由，两种补全都必须区分。
    let a = parse("a & (b | !b)");
    let b = parse("0");
    let binary: HashSet<String> = ["b"].iter().map(|s| s.to_string()).collect();
    let witness = minimal_counterexample(&a, &b, Algebra::Kleene, &binary)
        .expect("SAT 不应报错")
        .expect("应不等价");
    for bit in [false, true] {
        let mut asg = witness.clone();
        asg.set("b", Ternary::from_bool(bit));
        assert_ne!(
            eval(&a, &asg, Algebra::Kleene),
            eval(&b, &asg, Algebra::Kleene),
            "b={bit} 的补全也必须区分"
        );
    }
}

#[test]
fn binary_fast_path_uses_strong_add_not_xor() {
    // 常量走二值快速路径；⊕ 在 K3/B3 上是 ∧，在 Ł3 上是 ∨。
    for (alg, left, right) in [
        (Algebra::Kleene, "1 ^ 1", "1"),
        (Algebra::Kleene, "1 ^ 0", "0"),
        (Algebra::Kleene, "0 ^ 1", "0"),
        (Algebra::Lukasiewicz, "1 ^ 0", "1"),
        (Algebra::Lukasiewicz, "0 ^ 0", "0"),
        (Algebra::Bochvar, "1 ^ 1", "1"),
    ] {
        let a = parse(left);
        let b = parse(right);
        assert!(
            equivalent(&a, &b, alg).equivalent,
            "{alg:?}: {left} vs {right} 应当等价"
        );
        let sat = equivalent_symbolic(&a, &b, alg).expect("SAT 编码不应失败");
        assert!(sat.equivalent, "{alg:?}: {left} vs {right} SAT 误判");
    }

    // 声明为二值的变量也走快速路径。
    let binary: HashSet<String> = ["a".to_string()].into_iter().collect();
    let a = parse("a ^ 1");
    let k3 = equivalent_symbolic_with(&a, &parse("a"), Algebra::Kleene, &binary)
        .expect("SAT 编码不应失败");
    assert!(k3.equivalent, "K3: a ^ 1 ≡ a");
    let l3 = equivalent_symbolic_with(&a, &parse("1"), Algebra::Lukasiewicz, &binary)
        .expect("SAT 编码不应失败");
    assert!(l3.equivalent, "Ł3: a ^ 1 ≡ 1");
}
