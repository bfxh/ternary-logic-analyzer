//! 组合时序估算（STA-lite）：单位门延迟、输入到达 0、按拓扑序传播。

use std::collections::BTreeMap;

use crate::circuit::{Circuit, CircuitError, topological_order, validate};

/// 时序模型：所有门同一延迟（常量门为 0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimingModel {
    /// 每个非常量门的延迟。
    pub delay: u32,
}

impl Default for TimingModel {
    fn default() -> Self {
        Self { delay: 1 }
    }
}

/// 单个主输出的时序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputTiming {
    /// 输出名。
    pub name: String,
    /// 到达时间。
    pub arrival: u32,
    /// 要求时间（默认取关键路径）。
    pub required: u32,
    /// 裕量；负数即违例。
    pub slack: i64,
}

/// 时序报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimingReport {
    /// 最长组合路径（所有输出到达时间的最大值）。
    pub critical_path: u32,
    /// 使用的时钟周期。
    pub clock: u32,
    /// 每个主输出。
    pub outputs: Vec<OutputTiming>,
    /// 违例输出数。
    pub violations: usize,
}

/// 计算组合时序。
pub fn analyze_timing(
    circuit: &Circuit,
    model: TimingModel,
    clock: Option<u32>,
) -> Result<TimingReport, CircuitError> {
    validate(circuit)?;
    let order = topological_order(circuit)?;
    let mut arrival: BTreeMap<String, u32> = circuit
        .inputs
        .iter()
        .map(|signal| (signal.name().to_string(), 0))
        .collect();
    for &index in &order {
        let gate = &circuit.gates[index];
        let start = gate
            .inputs
            .iter()
            .map(|signal| arrival.get(signal.name()).copied().unwrap_or(0))
            .max()
            .unwrap_or(0);
        let delay = if gate.kind == crate::circuit::GateKind::Const {
            0
        } else {
            model.delay
        };
        arrival.insert(gate.output.name().to_string(), start.saturating_add(delay));
    }
    let critical_path = circuit
        .outputs
        .iter()
        .map(|signal| arrival.get(signal.name()).copied().unwrap_or(0))
        .max()
        .unwrap_or(0);
    let clock = clock.unwrap_or(critical_path);
    let mut required: BTreeMap<String, u32> = circuit
        .outputs
        .iter()
        .map(|signal| (signal.name().to_string(), clock))
        .collect();
    for &index in order.iter().rev() {
        let gate = &circuit.gates[index];
        let output_required = required.get(gate.output.name()).copied().unwrap_or(clock);
        let delay = if gate.kind == crate::circuit::GateKind::Const {
            0
        } else {
            model.delay
        };
        let input_required = output_required.saturating_sub(delay);
        for input in &gate.inputs {
            let entry = required
                .entry(input.name().to_string())
                .or_insert(input_required);
            if input_required < *entry {
                *entry = input_required;
            }
        }
    }
    let outputs: Vec<OutputTiming> = circuit
        .outputs
        .iter()
        .map(|signal| {
            let arrival = arrival.get(signal.name()).copied().unwrap_or(0);
            let required = required.get(signal.name()).copied().unwrap_or(clock);
            OutputTiming {
                name: signal.name().to_string(),
                arrival,
                required,
                slack: required as i64 - arrival as i64,
            }
        })
        .collect();
    let violations = outputs.iter().filter(|output| output.slack < 0).count();
    Ok(TimingReport {
        critical_path,
        clock,
        outputs,
        violations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::{Gate, GateKind, Signal};
    use crate::netlist::parse_netlist;

    #[test]
    fn chain_delay_and_clock_slack() {
        let circuit =
            parse_netlist("input a\ninput b\ninput c\ngate AND a b n\ngate AND n c y\noutput y\n")
                .unwrap();
        let report = analyze_timing(&circuit, TimingModel::default(), None).unwrap();
        assert_eq!(report.critical_path, 2);
        assert_eq!(report.clock, 2);
        assert_eq!(report.outputs[0].slack, 0);
        let tight = analyze_timing(&circuit, TimingModel::default(), Some(1)).unwrap();
        assert_eq!(tight.outputs[0].slack, -1);
        assert_eq!(tight.violations, 1);
        let loose = analyze_timing(&circuit, TimingModel::default(), Some(4)).unwrap();
        assert_eq!(loose.outputs[0].slack, 2);
        assert_eq!(loose.violations, 0);
    }

    #[test]
    fn constant_gate_has_zero_delay() {
        let mut circuit = Circuit::new("c");
        circuit.add_input(Signal::new("a"));
        circuit.add_gate(Gate::constant(
            trit_core::value::Ternary::True,
            Signal::new("one"),
        ));
        circuit.add_gate(Gate::new(
            GateKind::Buf,
            vec![Signal::new("a")],
            Signal::new("y"),
        ));
        circuit.add_output(Signal::new("y"));
        circuit.add_output(Signal::new("one"));
        let report = analyze_timing(&circuit, TimingModel::default(), None).unwrap();
        assert_eq!(report.critical_path, 1);
    }
}
