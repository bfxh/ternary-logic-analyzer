use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::parser::Parser;
use trit_eval::eval::{decode_assignment, equivalent, eval};
use trit_sat::symbolic::equivalent_symbolic;

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
