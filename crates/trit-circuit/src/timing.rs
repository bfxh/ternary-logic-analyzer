//! 组合时序估算（STA-lite）：单位门延迟、输入到达 0、按拓扑序传播。

use std::collections::BTreeMap;

use crate::circuit::{Circuit, CircuitError, GateKind, topological_order, validate};

/// 时序模型：每类门一个延迟；未列出的用 `default_delay`（常量门恒为 0）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TimingModel {
    /// 未在 `delays` 里列出的门的延迟。
    pub default_delay: u32,
    /// 门类型（`AND` / `OR` / ...）到延迟的映射。
    pub delays: BTreeMap<String, u32>,
}

impl Default for TimingModel {
    fn default() -> Self {
        Self {
            default_delay: 1,
            delays: BTreeMap::new(),
        }
    }
}

impl TimingModel {
    /// 某类门的延迟；常量门为 0。
    pub fn delay_for(&self, kind: GateKind) -> u32 {
        if kind == GateKind::Const {
            return 0;
        }
        self.delays
            .get(kind.as_str())
            .copied()
            .unwrap_or(self.default_delay)
    }

    /// 从 JSON 读取（形如 `{"default_delay":1,"delays":{"AND":2}}`）。
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// 序列化为 JSON。
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
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

/// 一条时序路径（信号序列，从主输入/常量到主输出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimingPath {
    /// 终点主输出。
    pub output: String,
    /// 信号序列。
    pub signals: Vec<String>,
    /// 路径延迟。
    pub delay: u32,
}

/// 计算每个输出最多 `limit` 条最慢路径（DAG 上的 k-best 动态规划）。
pub fn critical_paths(
    circuit: &Circuit,
    model: TimingModel,
    limit: usize,
) -> Result<Vec<TimingPath>, CircuitError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    validate(circuit)?;
    let order = topological_order(circuit)?;
    let mut best: BTreeMap<String, Vec<(u32, Vec<String>)>> = BTreeMap::new();
    for input in &circuit.inputs {
        best.insert(
            input.name().to_string(),
            vec![(0, vec![input.name().to_string()])],
        );
    }
    for &index in &order {
        let gate = &circuit.gates[index];
        let output = gate.output.name().to_string();
        let delay = model.delay_for(gate.kind);
        let mut candidates: Vec<(u32, Vec<String>)> = Vec::new();
        if gate.kind == GateKind::Const {
            candidates.push((0, vec![output.clone()]));
        } else {
            for input in &gate.inputs {
                if let Some(paths) = best.get(input.name()) {
                    for (path_delay, signals) in paths {
                        let mut path = signals.clone();
                        path.push(output.clone());
                        candidates.push((path_delay.saturating_add(delay), path));
                    }
                }
            }
        }
        candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        candidates.truncate(limit);
        best.insert(output, candidates);
    }
    let mut paths: Vec<TimingPath> = Vec::new();
    for output in &circuit.outputs {
        for (delay, signals) in best.get(output.name()).cloned().unwrap_or_default() {
            paths.push(TimingPath {
                output: output.name().to_string(),
                signals,
                delay,
            });
        }
    }
    paths.sort_by(|left, right| {
        right
            .delay
            .cmp(&left.delay)
            .then_with(|| left.signals.cmp(&right.signals))
    });
    paths.truncate(limit);
    Ok(paths)
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
        arrival.insert(
            gate.output.name().to_string(),
            start.saturating_add(model.delay_for(gate.kind)),
        );
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
        let input_required = output_required.saturating_sub(model.delay_for(gate.kind));
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

    #[test]
    fn per_kind_delays_and_json_model() {
        let circuit = parse_netlist(
            "input a
input b
input c
gate AND a b n
gate OR n c y
output y
",
        )
        .unwrap();
        let mut model = TimingModel::default();
        model.delays.insert("AND".to_string(), 2);
        model.delays.insert("OR".to_string(), 3);
        let report = analyze_timing(&circuit, model.clone(), None).unwrap();
        assert_eq!(report.critical_path, 5);

        let json = model.to_json().unwrap();
        let again = TimingModel::from_json(&json).unwrap();
        assert_eq!(again.delay_for(GateKind::And), 2);
        assert_eq!(again.delay_for(GateKind::Xor), 1);
    }

    #[test]
    fn critical_paths_enumerates_k_best() {
        let circuit = parse_netlist(
            "input a
input b
gate AND a b n
buf n m
gate OR a m y
output y
",
        )
        .unwrap();
        let paths = critical_paths(&circuit, TimingModel::default(), 3).unwrap();
        assert_eq!(paths.len(), 3);
        assert_eq!(paths[0].delay, 3);
        assert_eq!(paths[0].signals, vec!["a", "n", "m", "y"]);
        assert_eq!(paths[1].delay, 3);
        assert_eq!(paths[2].delay, 1);
        let top = critical_paths(&circuit, TimingModel::default(), 1).unwrap();
        assert_eq!(top.len(), 1);
    }
}
