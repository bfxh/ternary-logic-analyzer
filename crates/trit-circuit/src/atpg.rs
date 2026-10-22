//! 自动测试向量生成（ATPG）：对可观测的引脚 stuck-at 故障做贪心集合覆盖。

use std::collections::BTreeMap;

use trit_core::algebra::Algebra;
use trit_core::value::Ternary;

use crate::circuit::{Circuit, CircuitError, Fault, analyze};

/// 一个测试向量，以及它覆盖的故障下标（对应 [`AtpgReport::faults`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestPattern {
    /// 二进制主输入赋值。
    pub inputs: BTreeMap<String, Ternary>,
    /// 被这个向量首次覆盖的故障下标。
    pub covers: Vec<usize>,
}

/// 按门类型统计的覆盖率。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoverageStat {
    /// 该类型门的引脚故障总数。
    pub total: usize,
    /// 其中可测的数量。
    pub testable: usize,
}

impl CoverageStat {
    /// 覆盖率；没有故障时记为 1.0。
    pub fn coverage(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.testable as f64 / self.total as f64
        }
    }
}

/// ATPG 结果。
#[derive(Debug, Clone, PartialEq)]
pub struct AtpgReport {
    /// 全部门引脚故障（含不可测）。
    pub faults: Vec<Fault>,
    /// 贪心选出的测试向量。
    pub patterns: Vec<TestPattern>,
    /// 可测故障数。
    pub testable: usize,
    /// 不可测故障。
    pub untestable: Vec<Fault>,
    /// 覆盖率：`testable / faults`。
    pub coverage: f64,
    /// 按门类型（`And` / `Or` / ...）的覆盖统计。
    pub by_gate_kind: BTreeMap<&'static str, CoverageStat>,
    /// 是否对全部 `2^n` 向量做了覆盖搜索（否则只用了各故障的首个见证）。
    pub exhaustive: bool,
}

fn witness_map(faults: &[Fault]) -> BTreeMap<u64, Vec<usize>> {
    let mut by_mask: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (index, fault) in faults.iter().enumerate() {
        if let Some(mask) = fault.witness {
            by_mask.entry(mask).or_default().push(index);
        }
    }
    by_mask
}

/// 枚举全部二进制向量，记录每个向量能检出哪些故障。
fn full_coverage_matrix(
    circuit: &Circuit,
    alg: Algebra,
    faults: &[Fault],
) -> BTreeMap<u64, Vec<usize>> {
    let gate_of: BTreeMap<&str, usize> = circuit
        .gates
        .iter()
        .enumerate()
        .map(|(index, gate)| (gate.output.name(), index))
        .collect();
    let total = 1u64 << circuit.input_count();
    let mut by_mask: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for mask in 0..total {
        let inputs = circuit.decode_binary(mask);
        let good = crate::circuit::simulate_assignment(circuit, &inputs, alg, None);
        let mut covers = Vec::new();
        for (index, fault) in faults.iter().enumerate() {
            if !fault.observable {
                continue;
            }
            let Some(&gate_index) = gate_of.get(fault.gate.as_str()) else {
                continue;
            };
            let bad = crate::circuit::simulate_assignment(
                circuit,
                &inputs,
                alg,
                Some((gate_index, fault.input_index, fault.stuck_at)),
            );
            if circuit
                .outputs
                .iter()
                .any(|o| good.get(o.name()) != bad.get(o.name()))
            {
                covers.push(index);
            }
        }
        if !covers.is_empty() {
            by_mask.insert(mask, covers);
        }
    }
    by_mask
}

/// 生成 stuck-at 测试向量。
///
/// 小规模电路（`故障数 × 2^n ≤ 50000`）会枚举全部二进制向量，再做“覆盖最多
/// 未覆盖故障”的贪心；否则退回只用每个故障的首个见证。平局取更小的 mask。
pub fn generate_tests(circuit: &Circuit, alg: Algebra) -> Result<AtpgReport, CircuitError> {
    let report = analyze(circuit, alg)?;
    let faults = report.faults;
    let input_count = circuit.input_count();
    let testable_faults = faults.iter().filter(|f| f.observable).count();
    let exhaustive =
        input_count <= 10 && testable_faults.saturating_mul(1usize << input_count) <= 50_000;
    let by_mask = if exhaustive {
        full_coverage_matrix(circuit, alg, &faults)
    } else {
        witness_map(&faults)
    };
    let testable = faults.iter().filter(|f| f.observable).count();
    let mut uncovered: Vec<bool> = faults.iter().map(|f| f.observable).collect();
    let mut patterns: Vec<TestPattern> = Vec::new();

    loop {
        let mut best: Option<(u64, usize)> = None;
        for (&mask, covers) in &by_mask {
            let gain = covers.iter().filter(|&&i| uncovered[i]).count();
            if gain == 0 {
                continue;
            }
            let better = match best {
                None => true,
                Some((best_mask, best_gain)) => {
                    gain > best_gain || (gain == best_gain && mask < best_mask)
                }
            };
            if better {
                best = Some((mask, gain));
            }
        }
        let Some((mask, _)) = best else {
            break;
        };
        let mut covers = Vec::new();
        for &index in &by_mask[&mask] {
            if uncovered[index] {
                uncovered[index] = false;
                covers.push(index);
            }
        }
        patterns.push(TestPattern {
            inputs: circuit.decode_binary(mask),
            covers,
        });
    }

    let untestable: Vec<Fault> = faults.iter().filter(|f| !f.observable).cloned().collect();
    let coverage = if faults.is_empty() {
        1.0
    } else {
        testable as f64 / faults.len() as f64
    };
    let kind_of: BTreeMap<String, &'static str> = circuit
        .gates
        .iter()
        .map(|gate| (gate.output.name().to_string(), gate.kind.as_str()))
        .collect();
    let mut by_gate_kind: BTreeMap<&'static str, CoverageStat> = BTreeMap::new();
    for fault in &faults {
        let kind = kind_of.get(&fault.gate).copied().unwrap_or("UNKNOWN");
        let stat = by_gate_kind.entry(kind).or_default();
        stat.total += 1;
        if fault.observable {
            stat.testable += 1;
        }
    }
    Ok(AtpgReport {
        faults,
        patterns,
        testable,
        untestable,
        coverage,
        by_gate_kind,
        exhaustive,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::{Gate, GateKind, Signal};

    fn sig(name: &str) -> Signal {
        Signal::new(name)
    }

    fn and2() -> Circuit {
        let mut c = Circuit::new("and2");
        c.add_input(sig("a"));
        c.add_input(sig("b"));
        c.add_gate(Gate::new(
            GateKind::And,
            vec![sig("a"), sig("b")],
            sig("out"),
        ));
        c.add_output(sig("out"));
        c
    }

    #[test]
    fn and_gate_has_full_coverage() {
        let report = generate_tests(&and2(), Algebra::Kleene).unwrap();
        assert_eq!(report.faults.len(), 4);
        assert_eq!(report.testable, 4);
        assert!(report.untestable.is_empty());
        assert!((report.coverage - 1.0).abs() < 1e-9);
        let covered: usize = report.patterns.iter().map(|p| p.covers.len()).sum();
        assert_eq!(covered, 4, "每个故障至少被一个向量覆盖");
        assert!(report.patterns.len() <= 3, "两个输入最多 3 个向量");
        for pattern in &report.patterns {
            for value in pattern.inputs.values() {
                assert_ne!(*value, Ternary::Unknown, "测试向量必须是二进制");
            }
        }
    }

    #[test]
    fn unobserved_gate_faults_are_untestable() {
        let mut c = Circuit::new("dead");
        c.add_input(sig("a"));
        c.add_input(sig("b"));
        c.add_gate(Gate::new(GateKind::And, vec![sig("a"), sig("b")], sig("n")));
        c.add_gate(Gate::new(GateKind::Buf, vec![sig("a")], sig("out")));
        c.add_output(sig("out"));

        let report = generate_tests(&c, Algebra::Kleene).unwrap();
        assert_eq!(report.faults.len(), 6);
        assert_eq!(report.testable, 2, "BUF 的两个极性可测，AND 的四个不可测");
        assert_eq!(report.untestable.len(), 4);
        assert!(
            report
                .untestable
                .iter()
                .all(|f| f.reason == Some(crate::circuit::UntestableReason::Unobserved)),
            "AND 的门输出到不了主输出"
        );
        assert!((report.coverage - 2.0 / 6.0).abs() < 1e-9);
    }

    #[test]
    fn full_search_uses_multi_fault_vectors() {
        let mut c = Circuit::new("buffers");
        c.add_input(sig("a"));
        c.add_input(sig("b"));
        c.add_input(sig("c"));
        c.add_gate(Gate::new(GateKind::Buf, vec![sig("a")], sig("x")));
        c.add_gate(Gate::new(GateKind::Buf, vec![sig("b")], sig("y")));
        c.add_gate(Gate::new(GateKind::Buf, vec![sig("c")], sig("z")));
        c.add_output(sig("x"));
        c.add_output(sig("y"));
        c.add_output(sig("z"));

        let report = generate_tests(&c, Algebra::Kleene).unwrap();
        assert!(report.exhaustive);
        assert_eq!(report.faults.len(), 6);
        assert_eq!(report.testable, 6);
        assert_eq!(report.patterns.len(), 2, "全 0 与全 1 各覆盖 3 个故障");
        let covered: usize = report.patterns.iter().map(|p| p.covers.len()).sum();
        assert_eq!(covered, 6);
    }

    #[test]
    fn coverage_breaks_down_by_gate_kind() {
        let mut c = Circuit::new("dead");
        c.add_input(sig("a"));
        c.add_input(sig("b"));
        c.add_gate(Gate::new(GateKind::And, vec![sig("a"), sig("b")], sig("n")));
        c.add_gate(Gate::new(GateKind::Buf, vec![sig("a")], sig("out")));
        c.add_output(sig("out"));

        let report = generate_tests(&c, Algebra::Kleene).unwrap();
        let and = report.by_gate_kind.get("AND").copied().unwrap_or_default();
        assert_eq!((and.testable, and.total), (0, 4));
        assert_eq!(and.coverage(), 0.0);
        let buf = report.by_gate_kind.get("BUF").copied().unwrap_or_default();
        assert_eq!((buf.testable, buf.total), (2, 2));
        assert!((buf.coverage() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn patterns_really_detect_covered_faults() {
        let circuit = and2();
        let report = generate_tests(&circuit, Algebra::Kleene).unwrap();
        for pattern in &report.patterns {
            for &index in &pattern.covers {
                let fault = &report.faults[index];
                let gate_index = circuit
                    .gates
                    .iter()
                    .position(|g| g.output.name() == fault.gate)
                    .expect("故障应能定位到门");
                let good = crate::circuit::simulate_assignment(
                    &circuit,
                    &pattern.inputs,
                    Algebra::Kleene,
                    None,
                );
                let bad = crate::circuit::simulate_assignment(
                    &circuit,
                    &pattern.inputs,
                    Algebra::Kleene,
                    Some((gate_index, fault.input_index, fault.stuck_at)),
                );
                let differs = circuit
                    .outputs
                    .iter()
                    .any(|o| good.get(o.name()) != bad.get(o.name()));
                assert!(
                    differs,
                    "向量 {:?} 实际未检出故障 {}/{} SA-{}",
                    pattern.inputs,
                    fault.gate,
                    fault.input,
                    fault.stuck_at.as_char()
                );
            }
        }
    }
}
