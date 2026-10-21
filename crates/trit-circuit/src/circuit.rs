//! 三值门级电路（网表）分析与故障检测。
//!
//! 支持的门类型：`AND` / `OR` / `NOT` / `XOR`（强加法）/ `NAND` / `NOR` / `BUF` / `CONST`。
//!
//! 电路用信号图描述：节点是 `Signal`，`Gate` 把若干输入连到输出。
//! 分析器对每个输入组合做一次“X 传播”模拟，输出：
//!
//! - 每根信号在每组输入下的取值；
//! - **未被遮蔽的未知传播**：`X` 是否真的传到了输出；
//! - **冗余输入**：某输入改成相反值时输出不变（可做测试点削减）；
//! - **不可测故障**：某输入的取值翻转无法被输出观测到。

use std::collections::{BTreeMap, BTreeSet};

use trit_core::algebra::Algebra;
use trit_core::value::Ternary;

/// 电路中的一个信号（节点）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Signal(pub String);

impl Signal {
    /// 构造信号名。
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// 信号名。
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 门类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum GateKind {
    /// 与门 `∧`。
    And,
    /// 或门 `∨`。
    Or,
    /// 非门 `¬`。
    Not,
    /// 强加法门 `⊕`。
    Xor,
    /// 与非门。
    Nand,
    /// 或非门。
    Nor,
    /// 缓冲器（直连）。
    Buf,
    /// 常量源，输出恒为指定值。
    Const,
}

impl GateKind {
    /// 门类型的稳定短名。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::And => "AND",
            Self::Or => "OR",
            Self::Not => "NOT",
            Self::Xor => "XOR",
            Self::Nand => "NAND",
            Self::Nor => "NOR",
            Self::Buf => "BUF",
            Self::Const => "CONST",
        }
    }

    /// 解析门类型名（不区分大小写）。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "AND" => Some(Self::And),
            "OR" => Some(Self::Or),
            "NOT" => Some(Self::Not),
            "XOR" => Some(Self::Xor),
            "NAND" => Some(Self::Nand),
            "NOR" => Some(Self::Nor),
            "BUF" => Some(Self::Buf),
            "CONST" => Some(Self::Const),
            _ => None,
        }
    }

    /// 门的输入元数（`CONST` 为 0，`NOT` / `BUF` 为 1，其余为 2）。
    pub const fn arity(self) -> usize {
        match self {
            Self::Const => 0,
            Self::Not | Self::Buf => 1,
            _ => 2,
        }
    }

    /// 计算门的输出。
    pub fn eval(self, inputs: &[Ternary], alg: Algebra) -> Ternary {
        match self {
            Self::Const => inputs.first().copied().unwrap_or(Ternary::False),
            Self::Buf => inputs.first().copied().unwrap_or(Ternary::Unknown),
            Self::Not => alg.not(inputs.first().copied().unwrap_or(Ternary::Unknown)),
            Self::And => alg.and(
                inputs.first().copied().unwrap_or(Ternary::Unknown),
                inputs.get(1).copied().unwrap_or(Ternary::Unknown),
            ),
            Self::Or => alg.or(
                inputs.first().copied().unwrap_or(Ternary::Unknown),
                inputs.get(1).copied().unwrap_or(Ternary::Unknown),
            ),
            Self::Xor => alg.strong_add(
                inputs.first().copied().unwrap_or(Ternary::Unknown),
                inputs.get(1).copied().unwrap_or(Ternary::Unknown),
            ),
            Self::Nand => alg.not(alg.and(
                inputs.first().copied().unwrap_or(Ternary::Unknown),
                inputs.get(1).copied().unwrap_or(Ternary::Unknown),
            )),
            Self::Nor => alg.not(alg.or(
                inputs.first().copied().unwrap_or(Ternary::Unknown),
                inputs.get(1).copied().unwrap_or(Ternary::Unknown),
            )),
        }
    }
}

/// 一条电路连接：门 → 输出信号，输入为信号列表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    /// 门类型。
    pub kind: GateKind,
    /// 输入信号。
    pub inputs: Vec<Signal>,
    /// 输出信号。
    pub output: Signal,
    /// 常量门的取值。
    pub const_value: Option<Ternary>,
}

impl Gate {
    /// 构造普通门。
    pub fn new(kind: GateKind, inputs: Vec<Signal>, output: Signal) -> Self {
        Self {
            kind,
            inputs,
            output,
            const_value: None,
        }
    }

    /// 构造常量门。
    pub fn constant(value: Ternary, output: Signal) -> Self {
        Self {
            kind: GateKind::Const,
            inputs: vec![output.clone()],
            output,
            const_value: Some(value),
        }
    }
}

/// 门输出与输入信号构成的一对（用于冗余输入 / 不可测故障报告）。
pub type GateSignalPair = (String, String);

/// 故障分析结果：冗余输入列表与不可测故障列表。
pub type FaultReport = (Vec<GateSignalPair>, Vec<GateSignalPair>);

/// 一个电路。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Circuit {
    /// 电路名。
    pub name: String,
    /// 外部输入（主输入）。
    pub inputs: Vec<Signal>,
    /// 外部输出（主输出）。
    pub outputs: Vec<Signal>,
    /// 门列表。
    pub gates: Vec<Gate>,
}

impl Circuit {
    /// 新建电路。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// 添加主输入。
    pub fn add_input(&mut self, sig: Signal) -> &mut Self {
        self.inputs.push(sig);
        self
    }

    /// 添加主输出。
    pub fn add_output(&mut self, sig: Signal) -> &mut Self {
        self.outputs.push(sig);
        self
    }

    /// 添加门。
    pub fn add_gate(&mut self, gate: Gate) -> &mut Self {
        self.gates.push(gate);
        self
    }

    /// 主输入个数。
    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }

    /// 全部输入组合数 `3^n`。
    pub fn combination_count(&self) -> u64 {
        3u64.saturating_pow(self.input_count() as u32)
    }
}

/// 一次输入组合下的信号取值表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulationRow {
    /// 主输入取值。
    pub inputs: BTreeMap<String, Ternary>,
    /// 内部与输出信号取值。
    pub signals: BTreeMap<String, Ternary>,
    /// 主输出取值。
    pub outputs: BTreeMap<String, Ternary>,
}

/// 电路分析报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CircuitReport {
    /// 电路名。
    pub circuit: String,
    /// 使用的语义。
    pub algebra: Algebra,
    /// 主输入个数。
    pub input_count: usize,
    /// 主输出个数。
    pub output_count: usize,
    /// 门个数。
    pub gate_count: usize,
    /// 主输出含未知（X）的组合数。
    pub unknown_output_combinations: u64,
    /// 冗余输入：`(门输出, 输入信号)`，该输入翻转不改变输出。
    pub redundant_inputs: Vec<GateSignalPair>,
    /// 不可测故障：`(门输出, 输入信号)`，该输入的 stuck-at 故障无法被观测。
    pub untestable_faults: Vec<GateSignalPair>,
    /// 每个主输出的可达取值集合。
    pub output_domains: BTreeMap<String, BTreeSet<Ternary>>,
}

/// 电路分析错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CircuitError {
    /// 电路存在组合环。
    #[error("电路存在组合环，无法求值")]
    CombinationalLoop,
    /// 门输入个数与门类型不符。
    #[error("门 {output} 的类型 {kind} 需要 {expected} 个输入，实际给了 {actual} 个")]
    ArityMismatch {
        /// 输出信号名。
        output: String,
        /// 门类型。
        kind: String,
        /// 期望输入数。
        expected: usize,
        /// 实际输入数。
        actual: usize,
    },
    /// 信号既未声明为输入，也没有驱动它的门。
    #[error("信号 {0} 未被任何门驱动，也不是主输入")]
    UndrivenSignal(String),
}

/// 对电路做一次完整分析。
pub fn analyze(circuit: &Circuit, alg: Algebra) -> Result<CircuitReport, CircuitError> {
    validate(circuit)?;

    let total = circuit.combination_count();
    let mut unknown_outputs = 0u64;
    let mut output_domains: BTreeMap<String, BTreeSet<Ternary>> = BTreeMap::new();
    for o in &circuit.outputs {
        output_domains.insert(o.name().to_string(), BTreeSet::new());
    }

    for combo in 0..total {
        let mut values: BTreeMap<String, Ternary> = BTreeMap::new();
        // 置入主输入
        let mut t = combo;
        for inp in &circuit.inputs {
            let digit = t % 3;
            t /= 3;
            let v = match digit {
                0 => Ternary::False,
                1 => Ternary::True,
                _ => Ternary::Unknown,
            };
            values.insert(inp.name().to_string(), v);
        }
        // 门求值（按拓扑序；无环时 gates 顺序即可）
        for gate in &circuit.gates {
            let ins: Vec<Ternary> = gate
                .inputs
                .iter()
                .map(|s| values.get(s.name()).copied().unwrap_or(Ternary::Unknown))
                .collect();
            let v = if gate.kind == GateKind::Const {
                gate.const_value.unwrap_or(Ternary::False)
            } else {
                gate.kind.eval(&ins, alg)
            };
            values.insert(gate.output.name().to_string(), v);
        }

        let mut any_unknown = false;
        for o in &circuit.outputs {
            let v = values.get(o.name()).copied().unwrap_or(Ternary::Unknown);
            if v.is_unknown() {
                any_unknown = true;
            }
            output_domains
                .entry(o.name().to_string())
                .or_default()
                .insert(v);
        }
        if any_unknown {
            unknown_outputs += 1;
        }
    }

    let (redundant, untestable) = detect_input_faults(circuit, alg)?;

    Ok(CircuitReport {
        circuit: circuit.name.clone(),
        algebra: alg,
        input_count: circuit.input_count(),
        output_count: circuit.outputs.len(),
        gate_count: circuit.gates.len(),
        unknown_output_combinations: unknown_outputs,
        redundant_inputs: redundant,
        untestable_faults: untestable,
        output_domains,
    })
}

/// 检查电路结构合法性。
fn validate(circuit: &Circuit) -> Result<(), CircuitError> {
    for gate in &circuit.gates {
        let actual = if gate.kind == GateKind::Const {
            0
        } else {
            gate.inputs.len()
        };
        if actual != gate.kind.arity() {
            return Err(CircuitError::ArityMismatch {
                output: gate.output.name().to_string(),
                kind: gate.kind.as_str().to_string(),
                expected: gate.kind.arity(),
                actual,
            });
        }
    }
    // 检查组合环与未驱动信号
    let mut driven: BTreeSet<String> = circuit
        .inputs
        .iter()
        .map(|s| s.name().to_string())
        .collect();
    for gate in &circuit.gates {
        driven.insert(gate.output.name().to_string());
    }
    for gate in &circuit.gates {
        for sig in &gate.inputs {
            if !driven.contains(sig.name()) {
                return Err(CircuitError::UndrivenSignal(sig.name().to_string()));
            }
        }
    }
    // 拓扑排序检测环
    if !is_acyclic(circuit) {
        return Err(CircuitError::CombinationalLoop);
    }
    Ok(())
}

/// 用 Kahn 算法判断电路是否为有向无环图。
fn is_acyclic(circuit: &Circuit) -> bool {
    // 被门驱动的信号集合：这些信号不是独立源，其取值依赖上游门
    let driven: BTreeSet<String> = circuit
        .gates
        .iter()
        .map(|g| g.output.name().to_string())
        .collect();
    // 节点集合 = 所有门输出 + 所有被驱动的内部信号
    let mut nodes: BTreeSet<String> = BTreeSet::new();
    for gate in &circuit.gates {
        nodes.insert(gate.output.name().to_string());
        for sig in &gate.inputs {
            if driven.contains(sig.name()) {
                nodes.insert(sig.name().to_string());
            }
        }
    }
    // 边：内部信号 → 门输出
    let mut indeg: BTreeMap<String, usize> = nodes.iter().map(|n| (n.clone(), 0)).collect();
    let mut edges: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for gate in &circuit.gates {
        for sig in &gate.inputs {
            if driven.contains(sig.name()) {
                edges
                    .entry(sig.name().to_string())
                    .or_default()
                    .push(gate.output.name().to_string());
                *indeg.entry(gate.output.name().to_string()).or_insert(0) += 1;
            }
        }
    }
    let mut queue: Vec<String> = indeg
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(k, _)| k.clone())
        .collect();
    let mut visited = 0usize;
    while let Some(node) = queue.pop() {
        visited += 1;
        if let Some(nexts) = edges.get(&node) {
            for next in nexts {
                if let Some(d) = indeg.get_mut(next) {
                    *d = d.saturating_sub(1);
                    if *d == 0 {
                        queue.push(next.clone());
                    }
                }
            }
        }
    }
    visited == nodes.len()
}

/// 逐个输入做 stuck-at 故障注入，判定冗余与不可测故障。
///
/// 采用标准定义：
///
/// - **冗余输入**（冗余门）：把该输入固定为 0 与固定为 1 时，本门输出
///   完全相同 ⇒ 该输入对本门无影响，可从电路中删除。
/// - **不可测故障**：该输入的 stuck-at 故障在**所有**主输入组合下都不改变
///   任何主输出 ⇒ 故障无法被外部观测到。
///
/// 返回 `(冗余输入列表, 不可测故障列表)`，每项为 `(门输出, 输入信号)`。
fn detect_input_faults(circuit: &Circuit, alg: Algebra) -> Result<FaultReport, CircuitError> {
    let total = circuit.combination_count();
    let mut redundant: Vec<GateSignalPair> = Vec::new();
    let mut untestable: Vec<GateSignalPair> = Vec::new();

    for gate in &circuit.gates {
        if gate.kind == GateKind::Const {
            continue;
        }
        for probe in &gate.inputs {
            // 收集该输入在各组合下的取值与本门输出取值
            let mut all: Vec<(Ternary, Ternary, Vec<Ternary>)> = Vec::new();
            for combo in 0..total {
                let base = simulate_combo(circuit, combo, alg);
                let probe_val = base.get(probe.name()).copied().unwrap_or(Ternary::Unknown);
                let gate_out = base
                    .get(gate.output.name())
                    .copied()
                    .unwrap_or(Ternary::Unknown);
                let outs: Vec<Ternary> = circuit
                    .outputs
                    .iter()
                    .map(|o| base.get(o.name()).copied().unwrap_or(Ternary::Unknown))
                    .collect();
                all.push((probe_val, gate_out, outs));
            }

            // 直接做法：对该输入分别注入 stuck-at-0 与 stuck-at-1，比较本门输出
            let mut gate_out_same = true;
            let mut observable_any = false;
            for combo in 0..total {
                let base = simulate_combo(circuit, combo, alg);
                let base_out = base
                    .get(gate.output.name())
                    .copied()
                    .unwrap_or(Ternary::Unknown);
                let base_outs: Vec<Ternary> = circuit
                    .outputs
                    .iter()
                    .map(|o| base.get(o.name()).copied().unwrap_or(Ternary::Unknown))
                    .collect();

                for stuck in [Ternary::False, Ternary::True] {
                    let injected = simulate_with_fault(
                        circuit,
                        combo,
                        alg,
                        Some((probe, stuck)),
                        Ternary::Unknown,
                    );
                    let inj_out = injected
                        .get(gate.output.name())
                        .copied()
                        .unwrap_or(Ternary::Unknown);
                    if inj_out != base_out {
                        gate_out_same = false;
                    }
                    let inj_outs: Vec<Ternary> = circuit
                        .outputs
                        .iter()
                        .map(|o| injected.get(o.name()).copied().unwrap_or(Ternary::Unknown))
                        .collect();
                    if inj_outs != base_outs {
                        observable_any = true;
                    }
                }
            }
            if gate_out_same {
                redundant.push((gate.output.name().to_string(), probe.name().to_string()));
            }
            if !observable_any {
                untestable.push((gate.output.name().to_string(), probe.name().to_string()));
            }
        }
    }
    Ok((redundant, untestable))
}

/// 计算门的拓扑序（门索引列表）。
///
/// 电路已通过 `validate` 校验（无组合环），故排序一定成功；
/// 若出现异常则退化为原始顺序。
fn topological_order(circuit: &Circuit) -> Vec<usize> {
    let driven: BTreeSet<String> = circuit
        .gates
        .iter()
        .map(|g| g.output.name().to_string())
        .collect();
    let mut pending: Vec<(usize, usize, Vec<String>)> = Vec::new();
    for (i, gate) in circuit.gates.iter().enumerate() {
        let deps: Vec<String> = gate
            .inputs
            .iter()
            .filter(|s| driven.contains(s.name()))
            .map(|s| s.name().to_string())
            .collect();
        pending.push((i, deps.len(), deps));
    }
    let mut order: Vec<usize> = Vec::with_capacity(circuit.gates.len());
    let mut resolved: BTreeSet<String> = circuit
        .gates
        .iter()
        .map(|g| g.output.name().to_string())
        .collect();
    let mut guard = 0usize;
    while order.len() < circuit.gates.len() && guard <= circuit.gates.len() {
        guard += 1;
        for (i, _dep_count, deps) in &pending {
            if order.contains(i) {
                continue;
            }
            if deps.iter().all(|d| resolved.contains(d)) {
                order.push(*i);
                resolved.insert(circuit.gates[*i].output.name().to_string());
            }
        }
    }
    if order.len() != circuit.gates.len() {
        return (0..circuit.gates.len()).collect();
    }
    order
}

/// 按给定输入组合模拟整个电路，返回所有信号取值。
fn simulate_combo(circuit: &Circuit, combo: u64, alg: Algebra) -> BTreeMap<String, Ternary> {
    simulate_with_fault(circuit, combo, alg, None, Ternary::Unknown)
}

/// 模拟电路并注入一个 stuck-at 故障。
///
/// `fault` 为 `Some((信号, 取值))` 时，该信号的取值被强制为给定值：
/// - 若该信号是主输入，直接覆盖主输入取值；
/// - 若该信号由门驱动，则在**驱动点**覆盖（该门的计算结果被替换），
///   故障随后沿拓扑序向下游传播。
fn simulate_with_fault(
    circuit: &Circuit,
    combo: u64,
    alg: Algebra,
    fault: Option<(&Signal, Ternary)>,
    _unused: Ternary,
) -> BTreeMap<String, Ternary> {
    let mut values: BTreeMap<String, Ternary> = BTreeMap::new();
    let mut t = combo;
    for inp in &circuit.inputs {
        let digit = t % 3;
        t /= 3;
        let v = match digit {
            0 => Ternary::False,
            1 => Ternary::True,
            _ => Ternary::Unknown,
        };
        values.insert(inp.name().to_string(), v);
    }
    // 主输入上的故障：直接覆盖
    if let Some((sig, fv)) = fault
        && circuit.inputs.iter().any(|i| i.name() == sig.name())
    {
        values.insert(sig.name().to_string(), fv);
    }
    // 按拓扑序求值
    for idx in topological_order(circuit) {
        let gate = &circuit.gates[idx];
        // 驱动点故障注入
        if let Some((sig, fv)) = fault
            && gate.output.name() == sig.name()
            && !circuit.inputs.iter().any(|i| i.name() == sig.name())
        {
            values.insert(gate.output.name().to_string(), fv);
            continue;
        }
        let ins: Vec<Ternary> = gate
            .inputs
            .iter()
            .map(|s| values.get(s.name()).copied().unwrap_or(Ternary::Unknown))
            .collect();
        let v = if gate.kind == GateKind::Const {
            gate.const_value.unwrap_or(Ternary::False)
        } else {
            gate.kind.eval(&ins, alg)
        };
        values.insert(gate.output.name().to_string(), v);
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个简单电路：out = a AND b
    fn and_circuit() -> Circuit {
        let mut c = Circuit::new("and");
        c.add_input(Signal::new("a"));
        c.add_input(Signal::new("b"));
        c.add_gate(Gate::new(
            GateKind::And,
            vec![Signal::new("a"), Signal::new("b")],
            Signal::new("out"),
        ));
        c.add_output(Signal::new("out"));
        c
    }

    #[test]
    fn analyze_simple_and() {
        let c = and_circuit();
        let r = analyze(&c, Algebra::Kleene).unwrap();
        assert_eq!(r.input_count, 2);
        assert_eq!(r.output_count, 1);
        assert_eq!(r.gate_count, 1);
        // 输出域 {0, 1, X}
        let dom = r.output_domains.get("out").unwrap();
        assert_eq!(dom.len(), 3);
        // 9 种组合中，X 传播到输出的组合数
        assert!(r.unknown_output_combinations > 0);
    }

    #[test]
    fn detects_combinational_loop() {
        let mut c = Circuit::new("loop");
        c.add_input(Signal::new("a"));
        c.add_gate(Gate::new(
            GateKind::Not,
            vec![Signal::new("n")],
            Signal::new("a"),
        ));
        c.add_gate(Gate::new(
            GateKind::Not,
            vec![Signal::new("a")],
            Signal::new("n"),
        ));
        c.add_output(Signal::new("n"));
        let err = analyze(&c, Algebra::Kleene).unwrap_err();
        assert!(matches!(err, CircuitError::CombinationalLoop));
    }

    #[test]
    fn detects_arity_mismatch() {
        let mut c = Circuit::new("bad");
        c.add_input(Signal::new("a"));
        c.add_gate(Gate::new(
            GateKind::And,
            vec![Signal::new("a")],
            Signal::new("out"),
        ));
        c.add_output(Signal::new("out"));
        let err = analyze(&c, Algebra::Kleene).unwrap_err();
        assert!(matches!(err, CircuitError::ArityMismatch { .. }));
    }

    #[test]
    fn gate_kind_parsing() {
        assert_eq!(GateKind::parse("and"), Some(GateKind::And));
        assert_eq!(GateKind::parse("XOR"), Some(GateKind::Xor));
        assert_eq!(GateKind::parse("nope"), None);
    }
}
