//! CLI 集成测试：调用编译好的二进制，验证各子命令的实际行为。

use std::process::Command;

/// 二进制路径（由 Cargo 在集成测试时注入）。
fn trit_bin() -> std::path::PathBuf {
    // target/debug/trit.exe；CARGO_BIN_EXE_<name> 环境变量更可靠
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_trit") {
        return std::path::PathBuf::from(p);
    }
    let mut dir = std::env::current_exe().expect("current exe");
    dir.pop(); // deps
    if dir.ends_with("deps") {
        dir.pop();
    }
    dir.join(format!("trit{}", std::env::consts::EXE_SUFFIX))
}

/// 运行 CLI，返回 (stdout, stderr, 退出码)。
fn run(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(trit_bin())
        .args(args)
        .output()
        .expect("failed to run trit binary");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn atpg_reports_full_coverage_for_and() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/and2.net");
    let (stdout, stderr, code) = run(&["atpg", path.to_str().unwrap()]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("覆盖率 100.0%（4/4）"), "{stdout}");
    assert!(stdout.contains("测试向量"), "{stdout}");
}

#[test]
fn help_lists_all_subcommands() {
    let (stdout, _, code) = run(&["--help"]);
    assert_eq!(code, 0);
    for sub in [
        "eval", "table", "equiv", "quantify", "simplify", "circuit", "atpg", "timing", "netlist",
        "belnap", "repl",
    ] {
        assert!(stdout.contains(sub), "help missing {sub}");
    }
}

#[test]
fn belnap_eval_paraconsistent_conflict() {
    let (stdout, stderr, code) = run(&["belnap", "eval", "a & b", "-s", "a=N,b=B"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("=>  F"), "stdout={stdout}");
}

#[test]
fn belnap_equiv_reports_counterexample_and_exit_2() {
    let (stdout, _, code) = run(&["belnap", "equiv", "a | !a", "1"]);
    assert_eq!(code, 2, "stdout={stdout}");
    assert!(stdout.contains("反例"), "stdout={stdout}");

    let (stdout, _, code) = run(&["belnap", "equiv", "!(a & b)", "!a | !b"]);
    assert_eq!(code, 0, "stdout={stdout}");
    assert!(stdout.contains("16 组全等"), "stdout={stdout}");
}

#[test]
fn eval_computes_kleene_conjunction() {
    // Kleene: 1 ∧ X = X
    let (stdout, _, code) = run(&["eval", "a & b", "-s", "a=1,b=X"]);
    assert_eq!(code, 0, "stderr should be empty");
    assert_eq!(stdout.trim(), "X");
}

#[test]
fn eval_computes_lukasiewicz_strong_add() {
    // Łukasiewicz: X ⊕ X = 1
    let (stdout, _, code) = run(&["-a", "lukasiewicz", "eval", "a ^ b", "-s", "a=X,b=X"]);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "1");
}

#[test]
fn eval_computes_bochvar_poison() {
    // Bochvar: 0 ∧ X = X
    let (stdout, _, code) = run(&["-a", "bochvar", "eval", "a & b", "-s", "a=0,b=X"]);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "X");
}

#[test]
fn eval_rejects_bad_assignment() {
    let (_, stderr, code) = run(&["eval", "a", "-s", "a=2"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("未知取值"), "stderr was: {stderr}");
}

#[test]
fn eval_reports_parse_error() {
    let (_, stderr, code) = run(&["eval", "a & @"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("解析失败"), "stderr was: {stderr}");
}

#[test]
fn table_has_all_assignment_rows() {
    // 2 个变量 → 3^2 = 9 行
    let (stdout, _, code) = run(&["table", "a & b"]);
    assert_eq!(code, 0);
    let data_lines = stdout
        .lines()
        .filter(|l| l.starts_with('|') && !l.contains("---"))
        .count();
    // 表头 1 行 + 9 数据行
    assert_eq!(data_lines, 10, "stdout was:\n{stdout}");
}

#[test]
fn table_json_output_is_valid() {
    let (stdout, _, code) = run(&["table", "a | b", "-f", "json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(v["variables"], serde_json::json!(["a", "b"]));
    assert_eq!(v["rows"].as_array().unwrap().len(), 9);
}

#[test]
fn table_csv_output_has_header() {
    let (stdout, _, code) = run(&["table", "a & b", "-f", "csv"]);
    assert_eq!(code, 0);
    let mut lines = stdout.lines();
    assert_eq!(lines.next().unwrap(), "a,b,f");
    assert_eq!(lines.next().unwrap(), "0,0,0");
    assert_eq!(lines.count(), 8);
}

#[test]
fn table_markdown_output() {
    let (stdout, _, code) = run(&["table", "a & b", "-f", "markdown"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("| a | b | f |"), "stdout was:\n{stdout}");
}

#[test]
fn equiv_detects_equivalence() {
    let (stdout, _, code) = run(&["equiv", "a & b", "b & a"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("等价"), "stdout was:\n{stdout}");
}

#[test]
fn equiv_detects_difference_with_exit_code_two() {
    let (stdout, _, code) = run(&["equiv", "a & b", "a | b"]);
    assert_eq!(code, 2, "stdout was:\n{stdout}");
    assert!(stdout.contains("不等价"), "stdout was:\n{stdout}");
    assert!(stdout.contains("反例"), "stdout was:\n{stdout}");
}

#[test]
fn equiv_json_contains_counterexample() {
    let (stdout, _, code) = run(&["equiv", "a & b", "a | b", "-f", "json"]);
    assert_eq!(code, 2);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(v["equivalent"], serde_json::json!(false));
    assert!(v.get("counterexample").is_some());
}

#[test]
fn quantify_forall_excluded_middle_fails_on_ternary_domain() {
    let (stdout, _, code) = run(&["quantify", "a | !a", "--vars", "a", "--kind", "forall"]);
    assert_eq!(code, 2, "stdout was:\n{stdout}");
    assert!(stdout.contains("不恒真"), "stdout was:\n{stdout}");
    assert!(stdout.contains("反例：（空）"), "stdout was:\n{stdout}");
}

#[test]
fn quantify_forall_holds_on_binary_domain() {
    let (stdout, _, code) = run(&[
        "quantify", "a | !a", "--vars", "a", "--kind", "forall", "--binary", "a",
    ]);
    assert_eq!(code, 0, "stdout was:\n{stdout}");
    assert!(stdout.contains("恒真"), "stdout was:\n{stdout}");
}

#[test]
fn quantify_json_exposes_free_witness() {
    let (stdout, _, code) = run(&["quantify", "a & b", "--vars", "a", "--json"]);
    assert_eq!(code, 0, "stdout was:\n{stdout}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(v["quantifier"], serde_json::json!("exists"));
    assert_eq!(v["free"], serde_json::json!(["b"]));
    assert_eq!(v["witness"]["b"], serde_json::json!("1"));
}

#[test]
fn quantify_rejects_unknown_variable() {
    let (_, stderr, code) = run(&["quantify", "a & b", "--vars", "c"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("不在表达式里"), "stderr was:\n{stderr}");
}

#[test]
fn simplify_reports_equivalence() {
    let (stdout, _, code) = run(&["simplify", "a & 1 | b & 0"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("语义等价：是"), "stdout was:\n{stdout}");
}

#[test]
fn simplify_json_contains_original_and_simplified() {
    let (stdout, _, code) = run(&["simplify", "a & a", "-f", "json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert!(v["original"].is_string());
    assert!(v["simplified"].is_string());
    assert_eq!(v["equivalent"], serde_json::json!(true));
}

#[test]
fn circuit_analyzes_netlist() {
    let netlist =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/and_gate.net");
    let (stdout, _, code) = run(&["circuit", netlist.to_str().unwrap()]);
    assert_eq!(code, 0, "stdout was:\n{stdout}");
    assert!(stdout.contains("主输入 2 个"), "stdout was:\n{stdout}");
    assert!(stdout.contains("输出值域"), "stdout was:\n{stdout}");
}

#[test]
fn circuit_rejects_malformed_netlist() {
    let dir = std::env::temp_dir().join("trit-test-netlist");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bad.net");
    std::fs::write(&path, "input a\ngate FOO a b out\n").unwrap();
    let (_, stderr, code) = run(&["circuit", path.to_str().unwrap()]);
    assert_ne!(code, 0);
    assert!(stderr.contains("未知门类型"), "stderr was: {stderr}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn circuit_propagates_x_assignment() {
    let netlist =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/and_gate.net");
    let path = netlist.to_str().unwrap();

    let (stdout, _, code) = run(&["circuit", path, "--assign", "a=X,b=0"]);
    assert_eq!(code, 0, "stdout was:\n{stdout}");
    assert!(stdout.contains("y = 0"), "stdout was:\n{stdout}");
    assert!(stdout.contains("被遮蔽"), "stdout was:\n{stdout}");

    let (stdout, _, code) = run(&["circuit", path, "--assign", "a=X,b=1"]);
    assert_eq!(code, 0, "stdout was:\n{stdout}");
    assert!(stdout.contains("X 来源：a"), "stdout was:\n{stdout}");
}

#[test]
fn circuit_assign_requires_all_inputs() {
    let netlist =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/and_gate.net");
    let (_, stderr, code) = run(&["circuit", netlist.to_str().unwrap(), "--assign", "a=1"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("缺少主输入 b"), "stderr was: {stderr}");
}

#[test]
fn circuit_const_masks_x() {
    let netlist =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/x_mask.net");
    let (stdout, _, code) = run(&["circuit", netlist.to_str().unwrap(), "--assign", "a=X"]);
    assert_eq!(code, 0, "stdout was:\n{stdout}");
    assert!(stdout.contains("y_and = 0"), "stdout was:\n{stdout}");
    assert!(stdout.contains("y_or = 1"), "stdout was:\n{stdout}");
    assert!(stdout.contains("被遮蔽"), "stdout was:\n{stdout}");
}

#[test]
fn unknown_algebra_is_rejected() {
    let (_, stderr, code) = run(&["-a", "nonsense", "table", "a"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("未知的三值语义"), "stderr was: {stderr}");
}

#[test]
fn equiv_binary_fast_path_reports_stats() {
    let (stdout, stderr, code) = run(&[
        "equiv",
        "--symbolic",
        "--binary-all",
        "--stats",
        "a & b",
        "b & a",
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("等价"));
    assert!(stderr.contains("clauses="), "stderr={stderr}");
}

#[test]
fn binary_flag_requires_symbolic() {
    let (_, stderr, code) = run(&["equiv", "--binary", "a,b", "a & b", "b & a"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("只用于 --symbolic"), "stderr={stderr}");
}

#[test]
fn equiv_minimize_returns_relevant_vars_only() {
    let (stdout, stderr, code) = run(&[
        "equiv",
        "--symbolic",
        "--minimize",
        "--binary",
        "b",
        "a & (b | !b)",
        "0",
    ]);
    assert_eq!(code, 2, "stderr={stderr}");
    assert!(stdout.contains("最小反例：a="), "stdout={stdout}");
    assert!(!stdout.contains("b="), "b 应被最小化掉: {stdout}");
}

#[test]
fn minimize_requires_symbolic() {
    let (_, stderr, code) = run(&["equiv", "--minimize", "a", "!a"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("只用于 --symbolic"), "stderr={stderr}");
}

#[test]
fn equiv_bdd_works() {
    let (stdout, _, code) = run(&["equiv", "--bdd", "a & (b | c)", "(a & b) | (a & c)"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("等价"));
    let (stdout, _, code) = run(&["equiv", "--bdd", "a", "!a"]);
    assert_eq!(code, 2);
    assert!(stdout.contains("反例"));
}

#[test]
fn bdd_and_symbolic_conflict() {
    let (_, stderr, code) = run(&["equiv", "--bdd", "--symbolic", "a", "a"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("只能选一个"), "stderr={stderr}");
}

#[test]
fn bdd_stats_prints_nodes() {
    let (_, stderr, code) = run(&["equiv", "--bdd", "--stats", "a & b", "b & a"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stderr.contains("BDD：nodes="), "stderr={stderr}");
}

#[test]
fn verify_example_policy_passes() {
    let path = format!("{}/../../examples/policy.json", env!("CARGO_MANIFEST_DIR"));
    let (stdout, stderr, code) = run(&["verify", &path]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("OK"), "stdout={stdout}");
}

#[test]
fn verify_bad_policy_fails_with_counterexample() {
    let path = format!(
        "{}/tests/fixtures/bad_policy.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let (stdout, stderr, code) = run(&["verify", &path]);
    assert_eq!(code, 1, "stderr={stderr}");
    assert!(stdout.contains("FAILED"), "stdout={stdout}");
    assert!(stdout.contains("反例："), "stdout={stdout}");
}

#[test]
fn verify_bdd_engine_works() {
    let path = format!("{}/../../examples/policy.json", env!("CARGO_MANIFEST_DIR"));
    let (stdout, stderr, code) = run(&["verify", "--engine", "bdd", &path]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("OK"), "stdout={stdout}");
}

#[test]
fn quantify_accepts_bdd_order() {
    let (stdout, stderr, code) = run(&[
        "quantify",
        "(a & b) | (c & d)",
        "--vars",
        "a",
        "--bdd-order",
        "best",
        "--json",
    ]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("\"bdd_order\""), "{stdout}");
}

#[test]
fn atpg_csv_has_header_and_rows() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/and2.net");
    let (stdout, stderr, code) = run(&["atpg", path.to_str().unwrap(), "--format", "csv"]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("pattern,a,b,covered"));
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 3, "AND 门 3 个向量：{stdout}");
    assert!(
        rows.iter().all(|row| row.split(',').count() == 4),
        "{stdout}"
    );
}

#[test]
fn verify_accepts_bdd_order() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/admission.json");
    let (stdout, stderr, code) = run(&[
        "verify",
        path.to_str().unwrap(),
        "--engine",
        "bdd",
        "--bdd-order",
        "reverse",
    ]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("OK"), "{stdout}");
    assert!(stdout.contains("2 条断言"), "{stdout}");
}

#[test]
fn netlist_converts_text_to_json() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/and2.net");
    let (stdout, stderr, code) = run(&["netlist", path.to_str().unwrap()]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("\"inputs\""), "{stdout}");
    assert!(stdout.contains("\"And\""), "{stdout}");
}

#[test]
fn bdd_max_nodes_is_enforced() {
    let (stdout, stderr, code) = run(&[
        "equiv",
        "(a & b) | (c & d)",
        "(a & b) | (d & c)",
        "--bdd",
        "--bdd-max-nodes",
        "1",
    ]);
    assert_ne!(code, 0, "stdout={stdout}");
    assert!(stderr.contains("BDD 节点数超过上限"), "stderr={stderr}");
}

#[test]
fn verify_best_order_matches_appearance() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/admission.json");
    let (stdout, stderr, code) = run(&[
        "verify",
        path.to_str().unwrap(),
        "--engine",
        "bdd",
        "--bdd-order",
        "best",
    ]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("OK"), "{stdout}");
    assert!(stdout.contains("2 条断言"), "{stdout}");
}

#[test]
fn verify_best_rejects_sat_engine() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/admission.json");
    let (_, stderr, code) = run(&[
        "verify",
        path.to_str().unwrap(),
        "--engine",
        "sat",
        "--bdd-order",
        "best",
    ]);
    assert_ne!(code, 0);
    assert!(stderr.contains("只适用于 --engine bdd"), "{stderr}");
}

#[test]
fn netlist_converts_to_verilog() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/and2.net");
    let (stdout, stderr, code) = run(&["netlist", path.to_str().unwrap(), "--to", "verilog"]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("module netlist"), "{stdout}");
    assert!(stdout.contains("and g0"), "{stdout}");
}

#[test]
fn timing_reports_critical_path_and_slack() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chain3.net");
    let (stdout, stderr, code) = run(&["timing", path.to_str().unwrap(), "--clock", "1"]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("关键路径 2"), "{stdout}");
    assert!(stdout.contains("slack=-1"), "{stdout}");
    assert!(stdout.contains("违例 1"), "{stdout}");
}

#[test]
fn atpg_accepts_verilog() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/and2.v");
    let (stdout, stderr, code) = run(&["atpg", path.to_str().unwrap(), "--from", "verilog"]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("覆盖率 100.0%"), "{stdout}");
}
