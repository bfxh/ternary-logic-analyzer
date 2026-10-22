use trit_core::value::Ternary;
use trit_policy::{BddLimits, Engine, Policy, PolicyError, VarOrder};

const ADMISSION: &str = r#"{
  "algebra": "kleene",
  "binary": ["infeasible", "contended"],
  "definitions": { "reject": "infeasible | (hopeless & contended)" },
  "assertions": [
    { "name": "硬约束", "expr": "infeasible => reject" },
    { "name": "空槽不拒", "expr": "(!contended & !infeasible) => !reject" }
  ]
}"#;

#[test]
fn admission_policy_holds_on_both_engines() {
    let policy = Policy::from_json(ADMISSION).expect("策略应可解析");
    for engine in [Engine::Sat, Engine::Bdd] {
        let results = policy.verify(engine).expect("验证不应报错");
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.holds), "{engine:?}: {results:?}");
        assert!(results.iter().all(|r| r.counterexample.is_none()));
    }
}

#[test]
fn assumptions_filter_nondesignated_premise_values() {
    // 前提 a 只在 a=1 时成立；a=X 不满足前提，不能成为反例。
    let policy = Policy::from_json(
        r#"{
          "algebra": "kleene",
          "assumptions": ["a"],
          "assertions": [{ "name": "自身", "expr": "a" }]
        }"#,
    )
    .expect("策略应可解析");
    for engine in [Engine::Sat, Engine::Bdd] {
        let results = policy.verify(engine).expect("验证不应报错");
        assert!(results[0].holds, "{engine:?}: {:?}", results[0]);
    }
}

#[test]
fn counterexample_satisfies_premise() {
    let policy = Policy::from_json(
        r#"{
          "algebra": "kleene",
          "assumptions": ["a"],
          "assertions": [{ "name": "错误", "expr": "0" }]
        }"#,
    )
    .expect("策略应可解析");
    for engine in [Engine::Sat, Engine::Bdd] {
        let results = policy.verify(engine).expect("验证不应报错");
        assert!(!results[0].holds);
        let witness = results[0].counterexample.as_ref().expect("应有反例");
        assert_eq!(
            witness.get("a"),
            Some(Ternary::True),
            "{engine:?} 的反例必须满足前提"
        );
    }
}

#[test]
fn bad_assertion_returns_counterexample() {
    let policy = Policy::from_json(
        r#"{
          "binary": ["infeasible", "contended"],
          "definitions": { "reject": "infeasible | (hopeless & contended)" },
          "assertions": [{ "name": "无望必拒（错误断言）", "expr": "hopeless => reject" }]
        }"#,
    )
    .expect("策略应可解析");
    for engine in [Engine::Sat, Engine::Bdd] {
        let results = policy.verify(engine).expect("验证不应报错");
        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert!(!result.holds, "{engine:?} 应抓到反例：{result:?}");
        let witness = result.counterexample.as_ref().expect("应有反例");
        assert!(!witness.is_empty(), "反例不应为空");
        if engine == Engine::Bdd {
            assert!(result.nodes.is_some());
        }
    }
}

#[test]
fn definition_cycle_is_rejected() {
    let policy = Policy::from_json(
        r#"{
          "definitions": { "a": "b", "b": "a" },
          "assertions": [{ "name": "循环", "expr": "a" }]
        }"#,
    )
    .expect("策略应可解析");
    let err = policy.verify(Engine::Sat).unwrap_err();
    assert!(matches!(err, PolicyError::Cycle(_)), "{err:?}");
}

#[test]
fn unknown_fields_are_rejected() {
    let err = Policy::from_json(r#"{"albegra":"kleene","assertions":[{"name":"x","expr":"1"}]}"#)
        .unwrap_err();
    assert!(matches!(err, PolicyError::Json(_)), "{err:?}");
}

#[test]
fn empty_assertions_are_rejected() {
    let policy = Policy::from_json(r#"{"assertions":[]}"#).expect("JSON 结构合法");
    let err = policy.verify(Engine::Sat).unwrap_err();
    assert!(matches!(err, PolicyError::NoAssertions), "{err:?}");
}

#[test]
fn definitions_are_expanded() {
    let policy = Policy::from_json(
        r#"{
          "binary": ["a", "b"],
          "definitions": { "r": "a & b" },
          "assertions": [{ "name": "展开", "expr": "r => (a & b)" }]
        }"#,
    )
    .expect("策略应可解析");
    for engine in [Engine::Sat, Engine::Bdd] {
        let results = policy.verify(engine).expect("验证不应报错");
        assert!(results[0].holds, "{engine:?}: {results:?}");
    }
}

#[test]
fn invalid_json_is_an_error() {
    assert!(matches!(Policy::from_json("{"), Err(PolicyError::Json(_))));
}

#[test]
fn assumptions_restrict_the_domain() {
    let policy = Policy::from_json(
        r#"{
          "binary": ["infeasible", "contended", "hopeless"],
          "assumptions": ["contended"],
          "definitions": { "reject": "infeasible | (hopeless & contended)" },
          "assertions": [{ "name": "with contention", "expr": "hopeless => reject" }]
        }"#,
    )
    .expect("策略应可解析");
    for engine in [Engine::Sat, Engine::Bdd] {
        let results = policy.verify(engine).expect("验证不应报错");
        assert!(results[0].holds, "{engine:?}: {results:?}");
    }
}

#[test]
fn inconsistent_assumptions_are_rejected() {
    let policy = Policy::from_json(
        r#"{
          "binary": ["contended"],
          "assumptions": ["contended", "!contended"],
          "assertions": [{ "name": "unreachable", "expr": "0" }]
        }"#,
    )
    .expect("策略应可解析");
    for engine in [Engine::Sat, Engine::Bdd] {
        let err = policy.verify(engine).unwrap_err();
        assert!(
            matches!(err, PolicyError::InconsistentAssumptions),
            "{engine:?}: {err:?}"
        );
    }
}

#[test]
fn counterexample_respects_assumptions() {
    let policy = Policy::from_json(
        r#"{
          "binary": ["infeasible", "contended"],
          "assumptions": ["contended"],
          "definitions": { "reject": "infeasible | (hopeless & contended)" },
          "assertions": [{ "name": "wrong", "expr": "infeasible => !reject" }]
        }"#,
    )
    .expect("策略应可解析");
    let results = policy.verify(Engine::Sat).expect("验证不应报错");
    assert!(!results[0].holds);
    let witness = results[0].counterexample.as_ref().expect("应有反例");
    assert_eq!(
        witness.get("contended"),
        Some(Ternary::True),
        "反例必须满足前提"
    );
}

#[test]
fn bdd_order_does_not_change_admission_results() {
    let policy = Policy::from_json(ADMISSION).expect("策略应可解析");
    let base = policy.verify(Engine::Bdd).expect("验证不应报错");
    for order in [
        VarOrder::Reverse,
        VarOrder::FrequencyDesc,
        VarOrder::FrequencyAsc,
    ] {
        let got = policy
            .verify_with_bdd_order(Engine::Bdd, Some(order))
            .expect("验证不应报错");
        assert_eq!(got.len(), base.len(), "{}", order.as_str());
        for (left, right) in got.iter().zip(&base) {
            assert_eq!(left.holds, right.holds, "{}: {}", order.as_str(), left.name);
        }
    }
}

#[test]
fn bdd_budget_is_enforced_in_verify() {
    let policy = Policy::from_json(ADMISSION).expect("策略应可解析");
    let err = policy
        .verify_with_bdd_config(Engine::Bdd, None, BddLimits { max_nodes: 1 })
        .expect_err("1 个节点装不下");
    assert!(matches!(err, PolicyError::Bdd(_)), "{err}");
}
