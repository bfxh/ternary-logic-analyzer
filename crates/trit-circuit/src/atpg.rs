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
}

/// 生成 stuck-at 测试向量。
///
/// 每个可观测故障的 `witness` 已由分析阶段给出（首个二进制区分向量）；
/// 这里按“覆盖最多未覆盖故障”贪心选向量，平局取更小的 mask 以保证确定性。
pub fn generate_tests(circuit: &Circuit, alg: Algebra) -> Result<AtpgReport, CircuitError> {
    let report = analyze(circuit, alg)?;
    let faults = report.faults;
    let mut by_mask: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (index, fault) in faults.iter().enumerate() {
        if let Some(mask) = fault.witness {
            by_mask.entry(mask).or_default().push(index);
        }
    }
    let testable = by_mask.values().map(Vec::len).sum();
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
                let good = crate::circuit::simulate_for_test(
                    &circuit,
                    &pattern.inputs,
                    Algebra::Kleene,
                    None,
                );
                let bad = crate::circuit::simulate_for_test(
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
