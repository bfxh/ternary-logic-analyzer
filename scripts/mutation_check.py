#!/usr/bin/env python3
"""变异门：对关键规则做单点文本变异，跑对应回归；测试必须变红。

用法：python scripts/mutation_check.py
每个变异都会在 finally 里恢复原文件；退出码 1 表示有 mutant 存活（测试没抓住）。
"""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]

MUTANTS = [
    {
        "name": "core: K3 strong_add 用 OR 代替 AND",
        "file": "crates/trit-core/src/algebra.rs",
        "old": "    pub const fn strong_and(self, a: Ternary, b: Ternary) -> Ternary {\n        match self {\n            Self::Lukasiewicz => {\n                let s = a.as_f64() + b.as_f64() - 1.0;\n                if s <= 0.0 {\n                    Ternary::False\n                } else if s >= 1.0 {\n                    Ternary::True\n                } else {\n                    Ternary::Unknown\n                }\n            }\n            Self::Kleene | Self::Bochvar => {\n                if a.is_unknown() || b.is_unknown() {\n                    Ternary::Unknown\n                } else if a.is_true() && b.is_true() {",
        "new": "    pub const fn strong_and(self, a: Ternary, b: Ternary) -> Ternary {\n        match self {\n            Self::Lukasiewicz => {\n                let s = a.as_f64() + b.as_f64() - 1.0;\n                if s <= 0.0 {\n                    Ternary::False\n                } else if s >= 1.0 {\n                    Ternary::True\n                } else {\n                    Ternary::Unknown\n                }\n            }\n            Self::Kleene | Self::Bochvar => {\n                if a.is_unknown() || b.is_unknown() {\n                    Ternary::Unknown\n                } else if a.is_true() || b.is_true() {",
        "pkg": "trit-core",
        "filter": "strong_and_lukasiewicz_matches_formula",
    },
    {
        "name": "core: B3 或忘记扣掉未知位",
        "file": "crates/trit-core/src/bits.rs",
        "old": "let res1 = (a1 | b1) & !resx;",
        "new": "let res1 = a1 | b1;",
        "pkg": "trit-core",
        "filter": "ops_never_produce_illegal_rails",
    },
    {
        "name": "core: from_binary 不截断多余字",
        "file": "crates/trit-core/src/bulk.rs",
        "old": "        ones.truncate(words);",
        "new": "        let _ = words;",
        "pkg": "trit-core",
        "filter": "from_binary_normalizes",
    },
    {
        "name": "eval: OR 吸收误用 AND 形状",
        "file": "crates/trit-eval/src/simplify.rs",
        "old": "    if let Expr::And(p, q) = y",
        "new": "    if let Expr::Or(p, q) = y",
        "pkg": "trit-eval",
        "filter": "absorption_only_removes_matching_operator",
    },
    {
        "name": "eval: K3 a^a 不再折回 a",
        "file": "crates/trit-eval/src/simplify.rs",
        "old": "    if x == y && alg != Algebra::Lukasiewicz {",
        "new": "    if x == y && alg == Algebra::Lukasiewicz {",
        "pkg": "trit-eval",
        "filter": "xor_and_equiv_self_respect_unknown",
    },
    {
        "name": "eval: 自等价对所有代数折叠成 1",
        "file": "crates/trit-eval/src/simplify.rs",
        "old": "    if x == y && alg == Algebra::Lukasiewicz {",
        "new": "    if x == y {",
        "pkg": "trit-eval",
        "filter": "xor_and_equiv_self_respect_unknown",
    },
    {
        "name": "sat: 二值 strong_add 的 Ł3 分支用 AND",
        "file": "crates/trit-sat/src/symbolic.rs",
        "old": "            Algebra::Lukasiewicz => self.binary_or(a, b),",
        "new": "            Algebra::Lukasiewicz => self.binary_and(a, b),",
        "pkg": "trit-sat",
        "filter": "binary_fast_path_uses_strong_add_not_xor",
    },
    {
        "name": "circuit: 拓扑排序退回声明序",
        "file": "crates/trit-circuit/src/circuit.rs",
        "old": "            if deps.iter().all(|d| resolved.contains(d)) {",
        "new": "            if true {",
        "pkg": "trit-circuit",
        "filter": "evaluation_is_independent_of_gate_declaration_order",
    },
    {
        "name": "circuit: 引脚故障不注入",
        "file": "crates/trit-circuit/src/circuit.rs",
        "old": "            *slot = value;",
        "new": "            let _ = value;",
        "pkg": "trit-circuit",
        "filter": "fault_polarities_are_reported_separately",
    },
    {
        "name": "sat: 前提过滤不再要求 premise.t=1",
        "file": "crates/trit-sat/src/symbolic.rs",
        "old": "    enc.clause(&[Lit::from_var(p.t, true)]);",
        "new": "    enc.clause(&[Lit::from_var(p.t, false)]);",
        "pkg": "trit-policy",
        "filter": "counterexample_satisfies_premise",
    },
    {
        "name": "atpg: 全向量矩阵枚举为空",
        "file": "crates/trit-circuit/src/atpg.rs",
        "old": "    let total = 1u64 << circuit.input_count();",
        "new": "    let total = 0;",
        "pkg": "trit-circuit",
        "filter": "atpg",
    },
    {
        "name": "circuit: 不可观测误判为被遮蔽",
        "file": "crates/trit-circuit/src/circuit.rs",
        "old": "                } else if reachable.contains(gate.output.name()) {",
        "new": "                } else if true {",
        "pkg": "trit-circuit",
        "filter": "untestable_reasons",
    },
    {
        "name": "netlist: gate 参数计数失效",
        "file": "crates/trit-circuit/src/netlist.rs",
        "old": "                if parts.len() != 5 {",
        "new": "                if false {",
        "pkg": "trit-circuit",
        "filter": "reports_line_numbers",
    },
    {
        "name": "atpg: 精确覆盖递归错误清除已覆盖位",
        "file": "crates/trit-circuit/src/atpg.rs",
        "old": "        search_cover(uncovered & !faults, chosen, best, sets, max_gain, budget);",
        "new": "        search_cover(uncovered & faults, chosen, best, sets, max_gain, budget);",
        "pkg": "trit-circuit",
        "filter": "minimum_cover_matches_bruteforce",
    },
    {
        "name": "bdd: 节点预算永不触发",
        "file": "crates/trit-bdd/src/lib.rs",
        "old": "        if bdd.size() > self.limits.max_nodes {",
        "new": "        if false {",
        "pkg": "trit-bdd",
        "filter": "budget_rejects",
    },
    {
        "name": "bdd: 频率降序写反",
        "file": "crates/trit-bdd/src/lib.rs",
        "old": "                    cr.cmp(&cl)",
        "new": "                    cl.cmp(&cr)",
        "pkg": "trit-bdd",
        "filter": "frequency_order",
    },
    {
        "name": "bdd: is_valid 退回布尔全集",
        "file": "crates/trit-bdd/src/lib.rs",
        "old": "        self.bdd.not().and(&self.valid_free).is_false()",
        "new": "        self.bdd.is_true()",
        "pkg": "trit-bdd",
        "filter": "forall_with_free_ternary_variable_is_judged_on_valid_domain",
    },
    {
        "name": "bdd: satisfiable 不限制合法域",
        "file": "crates/trit-bdd/src/lib.rs",
        "old": "    let sat = enc.guard(enc.valid()?.and(&node.t).and(&node.x.not()))?;",
        "new": "    let sat = enc.guard(node.t.and(&node.x.not()))?;",
        "pkg": "trit-bdd",
        "filter": "satisfiable_requires_a_legal_valuation",
    },
]


def ran_any(output: str) -> bool:
    match = re.search(r"test result: ok\. (\d+) passed", output)
    return bool(match and int(match.group(1)) > 0)


def run_tests(pkg: str, test_filter: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["cargo", "test", "-q", "-p", pkg, test_filter, "--locked"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )


def main() -> int:
    baseline: dict[tuple[str, str], bool] = {}
    survived = 0
    for mutant in MUTANTS:
        key = (mutant["pkg"], mutant["filter"])
        if key not in baseline:
            result = run_tests(*key)
            if result.returncode != 0:
                print(f"BASELINE RED: {key}\n{result.stdout[-2000:]}")
                return 1
            baseline[key] = True

        path = ROOT / mutant["file"]
        original = path.read_text(encoding="utf-8")
        count = original.count(mutant["old"])
        if count != 1:
            print(f"BAD MUTANT {mutant['name']}: anchor 出现 {count} 次")
            return 1

        path.write_text(
            original.replace(mutant["old"], mutant["new"]), encoding="utf-8"
        )
        try:
            result = run_tests(*key)
        finally:
            path.write_text(original, encoding="utf-8")
            assert path.read_text(encoding="utf-8") == original, "恢复失败"

        if result.returncode == 0:
            survived += 1
            print(f"SURVIVED  {mutant['name']}")
        else:
            tail = next(
                (line for line in result.stdout.splitlines() if "FAILED" in line),
                "(有测试失败)",
            )
            print(f"caught    {mutant['name']}  [{tail.strip()}]")

    print(f"\n{len(MUTANTS) - survived}/{len(MUTANTS)} 个变异被抓住")
    return 1 if survived else 0


if __name__ == "__main__":
    sys.exit(main())
