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
//!
//! `propagate` 是单向量版本：给定一条含 `X` 的主输入赋值，只做一遍拓扑传播，
//! 报告每根信号取值、被遮蔽的门，以及每个输出上 `X` 的来源主输入。

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
            inputs: Vec::new(),
            output,
            const_value: Some(value),
        }
    }
}

/// 门输出与输入信号构成的一对（用于冗余输入 / 不可测故障报告）。
pub type GateSignalPair = (String, String);

/// 不可测故障的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UntestableReason {
    /// 固定该引脚不会改变本门输出。
    Redundant,
    /// 本门输出到不了任何主输出。
    Unobserved,
    /// 能到主输出，但被其他信号遮蔽。
    Masked,
}

impl UntestableReason {
    /// 稳定短名。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Redundant => "redundant",
            Self::Unobserved => "unobserved",
            Self::Masked => "masked",
        }
    }
}

/// 一个门引脚上的 stuck-at 故障。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// 门输出（用于定位门）。
    pub gate: String,
    /// 该引脚的输入信号。
    pub input: String,
    /// 输入序号（同一信号可以接多个引脚）。
    pub input_index: usize,
    /// 故障值：`False` = stuck-at-0，`True` = stuck-at-1。
    pub stuck_at: Ternary,
    /// 是否能被任一主输出观测到。
    pub observable: bool,
    /// 首个能区分故障与正常的**二进制**输入向量（bit i = 第 i 个主输入）；不可测时为 `None`。
    pub witness: Option<u64>,
    /// 不可测原因；可测时为 `None`。
    pub reason: Option<UntestableReason>,
}

/// 故障分析结果：冗余引脚列表与逐极性故障列表。
pub type FaultReport = (Vec<GateSignalPair>, Vec<Fault>);

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

    /// 把二进制测试向量（bit i 对应第 i 个主输入）解码成赋值表。
    pub fn decode_binary(&self, mask: u64) -> BTreeMap<String, Ternary> {
        self.inputs
            .iter()
            .enumerate()
            .map(|(i, sig)| {
                let value = if (mask >> i) & 1 == 0 {
                    Ternary::False
                } else {
                    Ternary::True
                };
                (sig.name().to_string(), value)
            })
            .collect()
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
    /// 逐引脚、逐极性的 stuck-at 故障（含不可测项）。
    pub faults: Vec<Fault>,
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
    /// 同一个信号被声明了多次主输入。
    #[error("信号 {0} 被重复声明为主输入")]
    DuplicateInput(String),
    /// 同一个信号被多个源驱动（门或主输入）。
    #[error("信号 {0} 有多个驱动源")]
    MultipleDrivers(String),
    /// 输入向量缺少主输入。
    #[error("输入向量缺少主输入 {0}")]
    MissingInput(String),
    /// 输入向量里出现了非主输入。
    #[error("输入向量包含非主输入信号 {0}")]
    UnknownInput(String),
    /// 主输入过多，二进制向量空间不可枚举。
    #[error("主输入 {0} 个，超过故障分析的 16 个上限")]
    TooManyInputs(usize),
}

/// 对电路做一次完整分析。
pub fn analyze(circuit: &Circuit, alg: Algebra) -> Result<CircuitReport, CircuitError> {
    validate(circuit)?;
    let order = topological_order(circuit)?;

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
        // 门求值：按真正的拓扑序，与声明顺序无关
        for &idx in &order {
            let gate = &circuit.gates[idx];
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

    let (redundant, faults) = detect_faults(circuit, alg)?;

    Ok(CircuitReport {
        circuit: circuit.name.clone(),
        algebra: alg,
        input_count: circuit.input_count(),
        output_count: circuit.outputs.len(),
        gate_count: circuit.gates.len(),
        unknown_output_combinations: unknown_outputs,
        redundant_inputs: redundant,
        faults,
        output_domains,
    })
}

/// 一条输入向量下的三值传播结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Propagation {
    /// 使用的语义。
    pub algebra: Algebra,
    /// 主输入取值（可含 `X`）。
    pub inputs: BTreeMap<String, Ternary>,
    /// 全部信号取值（主输入 + 门输出）。
    pub signals: BTreeMap<String, Ternary>,
    /// 主输出取值。
    pub outputs: BTreeMap<String, Ternary>,
    /// 主输出为 `X` 时，可能把 `X` 带进来的主输入。
    pub origins: BTreeMap<String, BTreeSet<String>>,
    /// 有 `X` 输入、但输出被遮蔽（或按代数确定）的门输出。
    pub masked: Vec<String>,
}

impl Propagation {
    /// 某输出是否为未知。
    pub fn output_is_unknown(&self, output: &str) -> bool {
        self.outputs.get(output).is_some_and(|v| v.is_unknown())
    }
}

/// 给定一条主输入赋值，做一遍三值传播。
///
/// - 必须给全所有主输入；缺一个就报 [`CircuitError::MissingInput`]。
/// - `masked` 记录“输入有 `X`、输出却确定”的门：K3 的 `X & 0` / `X | 1`、
///   Ł3 的 `X ⊕ X = 1` 都在内。
/// - `origins` 是**可能**来源（保守并集），不是最小依赖；已知输出为空集。
pub fn propagate(
    circuit: &Circuit,
    assignment: &BTreeMap<String, Ternary>,
    alg: Algebra,
) -> Result<Propagation, CircuitError> {
    validate(circuit)?;
    for inp in &circuit.inputs {
        if !assignment.contains_key(inp.name()) {
            return Err(CircuitError::MissingInput(inp.name().to_string()));
        }
    }
    for key in assignment.keys() {
        if !circuit.inputs.iter().any(|i| i.name() == key) {
            return Err(CircuitError::UnknownInput(key.clone()));
        }
    }

    let mut signals: BTreeMap<String, Ternary> = BTreeMap::new();
    let mut origins: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for inp in &circuit.inputs {
        let value = assignment[inp.name()];
        signals.insert(inp.name().to_string(), value);
        let from = if value.is_unknown() {
            BTreeSet::from([inp.name().to_string()])
        } else {
            BTreeSet::new()
        };
        origins.insert(inp.name().to_string(), from);
    }

    let mut masked: Vec<String> = Vec::new();
    for idx in topological_order(circuit)? {
        let gate = &circuit.gates[idx];
        let ins: Vec<Ternary> = gate
            .inputs
            .iter()
            .map(|s| signals.get(s.name()).copied().unwrap_or(Ternary::Unknown))
            .collect();
        let value = if gate.kind == GateKind::Const {
            gate.const_value.unwrap_or(Ternary::False)
        } else {
            gate.kind.eval(&ins, alg)
        };
        let inherited: BTreeSet<String> = gate
            .inputs
            .iter()
            .filter_map(|s| origins.get(s.name()))
            .flatten()
            .cloned()
            .collect();
        if value.is_unknown() {
            origins.insert(gate.output.name().to_string(), inherited);
        } else {
            origins.insert(gate.output.name().to_string(), BTreeSet::new());
            if ins.iter().any(|t| t.is_unknown()) {
                masked.push(gate.output.name().to_string());
            }
        }
        signals.insert(gate.output.name().to_string(), value);
    }

    let mut outputs: BTreeMap<String, Ternary> = BTreeMap::new();
    let mut output_origins: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for o in &circuit.outputs {
        outputs.insert(
            o.name().to_string(),
            signals.get(o.name()).copied().unwrap_or(Ternary::Unknown),
        );
        output_origins.insert(
            o.name().to_string(),
            origins.get(o.name()).cloned().unwrap_or_default(),
        );
    }

    Ok(Propagation {
        algebra: alg,
        inputs: assignment.clone(),
        signals,
        outputs,
        origins: output_origins,
        masked,
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
    let mut seen_inputs: BTreeSet<&str> = BTreeSet::new();
    for inp in &circuit.inputs {
        if !seen_inputs.insert(inp.name()) {
            return Err(CircuitError::DuplicateInput(inp.name().to_string()));
        }
    }
    let mut drivers: BTreeMap<&str, usize> = circuit.inputs.iter().map(|s| (s.name(), 1)).collect();
    for gate in &circuit.gates {
        *drivers.entry(gate.output.name()).or_insert(0) += 1;
    }
    if let Some((signal, _)) = drivers.iter().find(|(_, count)| **count > 1) {
        return Err(CircuitError::MultipleDrivers((*signal).to_string()));
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
    for out in &circuit.outputs {
        if !driven.contains(out.name()) {
            return Err(CircuitError::UndrivenSignal(out.name().to_string()));
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

/// 逐个**门引脚**做 stuck-at 故障注入，分别判定冗余与每个极性的可观测性。
///
/// 故障模型是 gate-pin：只替换该门该输入引脚上的取值，不覆盖整条共享信号。
/// 在**二进制**测试向量空间（`2^n`）上判定：
/// - 冗余引脚：把该引脚固定为 0 或 1，本门输出在所有向量下都相同；
/// - 可观测：存在一个二进制向量能让某个主输出与正常值不同（`witness` 记首个）。
///
/// X 输入不是物理测试向量，因此不计入可测性判定。
fn reachable_signals(circuit: &Circuit) -> BTreeSet<String> {
    let mut reachable: BTreeSet<String> = circuit
        .outputs
        .iter()
        .map(|s| s.name().to_string())
        .collect();
    loop {
        let mut changed = false;
        for gate in &circuit.gates {
            if !reachable.contains(gate.output.name()) {
                continue;
            }
            for input in &gate.inputs {
                if reachable.insert(input.name().to_string()) {
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    reachable
}

fn detect_faults(circuit: &Circuit, alg: Algebra) -> Result<FaultReport, CircuitError> {
    let input_count = circuit.input_count();
    if input_count > 16 {
        return Err(CircuitError::TooManyInputs(input_count));
    }
    let total = 1u64 << input_count;
    let order = topological_order(circuit)?;
    let reachable = reachable_signals(circuit);
    let mut redundant: Vec<GateSignalPair> = Vec::new();
    let mut faults: Vec<Fault> = Vec::new();

    let base: Vec<BTreeMap<String, Ternary>> = (0..total)
        .map(|mask| simulate_binary_with_fault(circuit, mask, alg, None, &order))
        .collect();

    for (gate_index, gate) in circuit.gates.iter().enumerate() {
        if gate.kind == GateKind::Const {
            continue;
        }
        for (input_index, probe) in gate.inputs.iter().enumerate() {
            let mut gate_values: [Vec<Ternary>; 2] = [Vec::new(), Vec::new()];
            let mut witness: [Option<u64>; 2] = [None, None];
            let mut fault_ids: Vec<usize> = Vec::with_capacity(2);
            for (polarity, stuck) in [Ternary::False, Ternary::True].into_iter().enumerate() {
                for mask in 0..total {
                    let injected = simulate_binary_with_fault(
                        circuit,
                        mask,
                        alg,
                        Some((gate_index, input_index, stuck)),
                        &order,
                    );
                    gate_values[polarity].push(
                        injected
                            .get(gate.output.name())
                            .copied()
                            .unwrap_or(Ternary::Unknown),
                    );
                    if witness[polarity].is_none()
                        && circuit.outputs.iter().any(|o| {
                            injected.get(o.name()).copied().unwrap_or(Ternary::Unknown)
                                != base[mask as usize]
                                    .get(o.name())
                                    .copied()
                                    .unwrap_or(Ternary::Unknown)
                        })
                    {
                        witness[polarity] = Some(mask);
                    }
                }
                fault_ids.push(faults.len());
                faults.push(Fault {
                    gate: gate.output.name().to_string(),
                    input: probe.name().to_string(),
                    input_index,
                    stuck_at: stuck,
                    observable: witness[polarity].is_some(),
                    witness: witness[polarity],
                    reason: None,
                });
            }
            let is_redundant = gate_values[0] == gate_values[1];
            if is_redundant {
                redundant.push((gate.output.name().to_string(), probe.name().to_string()));
            }
            for id in fault_ids {
                if faults[id].observable {
                    continue;
                }
                faults[id].reason = Some(if is_redundant {
                    UntestableReason::Redundant
                } else if reachable.contains(gate.output.name()) {
                    UntestableReason::Masked
                } else {
                    UntestableReason::Unobserved
                });
            }
        }
    }
    Ok((redundant, faults))
}

/// 计算门的拓扑序（门索引列表）。
///
/// 电路已通过 `validate` 校验（无组合环），故排序一定成功；
/// 若出现异常则退化为原始顺序。
fn topological_order(circuit: &Circuit) -> Result<Vec<usize>, CircuitError> {
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
    // 初始只能包含“不依赖任何门输出”的源；把全部门输出预置进来会让排序退化成声明序。
    let mut resolved: BTreeSet<String> = BTreeSet::new();
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
        return Err(CircuitError::CombinationalLoop);
    }
    Ok(order)
}

/// 按给定输入组合模拟整个电路，返回所有信号取值。
/// 二进制向量版本：`mask` 的 bit i 是第 i 个主输入（0/1）。
#[cfg(test)]
fn simulate_combo(
    circuit: &Circuit,
    combo: u64,
    alg: Algebra,
    order: &[usize],
) -> BTreeMap<String, Ternary> {
    let mut values: BTreeMap<String, Ternary> = BTreeMap::new();
    let mut t = combo;
    for inp in &circuit.inputs {
        let digit = t % 3;
        t /= 3;
        let value = match digit {
            0 => Ternary::False,
            1 => Ternary::True,
            _ => Ternary::Unknown,
        };
        values.insert(inp.name().to_string(), value);
    }
    evaluate_with_fault(circuit, values, alg, None, order)
}

fn simulate_binary_with_fault(
    circuit: &Circuit,
    mask: u64,
    alg: Algebra,
    fault: Option<(usize, usize, Ternary)>,
    order: &[usize],
) -> BTreeMap<String, Ternary> {
    evaluate_with_fault(circuit, circuit.decode_binary(mask), alg, fault, order)
}

/// 按拓扑序求值并注入一个 stuck-at 故障。
///
/// `fault` 为 `Some((门索引, 输入序号, 取值))`：只替换该门该输入引脚上的值，
/// 不覆盖整条共享信号，因此不会影响同一信号的其他扇出。
fn evaluate_with_fault(
    circuit: &Circuit,
    mut values: BTreeMap<String, Ternary>,
    alg: Algebra,
    fault: Option<(usize, usize, Ternary)>,
    order: &[usize],
) -> BTreeMap<String, Ternary> {
    // 按拓扑序求值
    for &idx in order {
        let gate = &circuit.gates[idx];
        let mut ins: Vec<Ternary> = gate
            .inputs
            .iter()
            .map(|s| values.get(s.name()).copied().unwrap_or(Ternary::Unknown))
            .collect();
        if let Some((gate_index, input_index, value)) = fault
            && gate_index == idx
            && let Some(slot) = ins.get_mut(input_index)
        {
            *slot = value;
        }
        let v = if gate.kind == GateKind::Const {
            gate.const_value.unwrap_or(Ternary::False)
        } else {
            gate.kind.eval(&ins, alg)
        };
        values.insert(gate.output.name().to_string(), v);
    }
    values
}

/// crate 内辅助：用给定赋值求值，可选注入门引脚故障。
pub(crate) fn simulate_assignment(
    circuit: &Circuit,
    inputs: &BTreeMap<String, Ternary>,
    alg: Algebra,
    fault: Option<(usize, usize, Ternary)>,
) -> BTreeMap<String, Ternary> {
    let order = topological_order(circuit).expect("测试电路应能拓扑排序");
    evaluate_with_fault(circuit, inputs.clone(), alg, fault, &order)
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

    fn sig(name: &str) -> Signal {
        Signal::new(name)
    }

    fn assignment(pairs: &[(&str, Ternary)]) -> BTreeMap<String, Ternary> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn propagate_masks_x_on_and_then_passes_it() {
        let c = and_circuit();
        let masked = propagate(
            &c,
            &assignment(&[("a", Ternary::Unknown), ("b", Ternary::False)]),
            Algebra::Kleene,
        )
        .unwrap();
        assert_eq!(masked.outputs["out"], Ternary::False);
        assert_eq!(masked.masked, vec!["out".to_string()]);
        assert!(masked.origins["out"].is_empty(), "已知输出不该有 X 来源");

        let passed = propagate(
            &c,
            &assignment(&[("a", Ternary::Unknown), ("b", Ternary::True)]),
            Algebra::Kleene,
        )
        .unwrap();
        assert_eq!(passed.outputs["out"], Ternary::Unknown);
        assert!(passed.masked.is_empty());
        assert_eq!(
            passed.origins["out"],
            BTreeSet::from(["a".to_string()]),
            "X 应只归因到 a"
        );
    }

    #[test]
    fn propagate_lukasiewicz_cancels_x_xor() {
        let mut c = Circuit::new("xor-self");
        c.add_input(sig("a"));
        c.add_gate(Gate::new(GateKind::Xor, vec![sig("a"), sig("a")], sig("y")));
        c.add_output(sig("y"));
        let assign = assignment(&[("a", Ternary::Unknown)]);

        let k3 = propagate(&c, &assign, Algebra::Kleene).unwrap();
        assert_eq!(k3.outputs["y"], Ternary::Unknown);
        assert!(k3.masked.is_empty());

        let l3 = propagate(&c, &assign, Algebra::Lukasiewicz).unwrap();
        assert_eq!(l3.outputs["y"], Ternary::True, "Ł3: X ⊕ X = 1");
        assert_eq!(l3.masked, vec!["y".to_string()]);
        assert!(l3.origins["y"].is_empty());
    }

    #[test]
    fn propagate_bochvar_poisons_known_zero() {
        let c = and_circuit();
        let p = propagate(
            &c,
            &assignment(&[("a", Ternary::Unknown), ("b", Ternary::False)]),
            Algebra::Bochvar,
        )
        .unwrap();
        assert_eq!(p.outputs["out"], Ternary::Unknown, "B3: X 毒化 0");
        assert!(p.masked.is_empty());
        assert_eq!(p.origins["out"], BTreeSet::from(["a".to_string()]));
    }

    #[test]
    fn propagate_requires_every_primary_input() {
        let c = and_circuit();
        let err = propagate(&c, &assignment(&[("a", Ternary::True)]), Algebra::Kleene).unwrap_err();
        assert!(matches!(err, CircuitError::MissingInput(ref s) if s == "b"));
    }

    #[test]
    fn propagate_matches_exhaustive_simulation() {
        let mut c = Circuit::new("mix");
        c.add_input(sig("a"));
        c.add_input(sig("b"));
        c.add_input(sig("c"));
        c.add_gate(Gate::new(GateKind::Xor, vec![sig("a"), sig("b")], sig("t")));
        c.add_gate(Gate::new(
            GateKind::Nand,
            vec![sig("t"), sig("c")],
            sig("u"),
        ));
        c.add_gate(Gate::new(GateKind::Not, vec![sig("u")], sig("y")));
        c.add_output(sig("y"));

        let order = topological_order(&c).unwrap();
        for combo in 0..c.combination_count() {
            let base = simulate_combo(&c, combo, Algebra::Kleene, &order);
            let assign: BTreeMap<String, Ternary> = c
                .inputs
                .iter()
                .map(|i| (i.name().to_string(), base[i.name()]))
                .collect();
            let p = propagate(&c, &assign, Algebra::Kleene).unwrap();
            for (name, value) in &p.signals {
                assert_eq!(
                    Some(value),
                    base.get(name),
                    "combo {combo} 信号 {name} 不一致"
                );
            }
        }
    }

    #[test]
    fn const_gate_masks_x_and_is_not_a_loop() {
        let mut c = Circuit::new("mask");
        c.add_input(sig("a"));
        c.add_gate(Gate::constant(Ternary::False, sig("zero")));
        c.add_gate(Gate::new(
            GateKind::And,
            vec![sig("a"), sig("zero")],
            sig("y"),
        ));
        c.add_output(sig("y"));

        let p = propagate(&c, &assignment(&[("a", Ternary::Unknown)]), Algebra::Kleene).unwrap();
        assert_eq!(p.outputs["y"], Ternary::False);
        assert_eq!(p.masked, vec!["y".to_string()]);
        analyze(&c, Algebra::Kleene).expect("Const 门不该被误判成组合环");
    }

    #[test]
    fn rejects_duplicate_drivers() {
        let mut c = Circuit::new("dup-driver");
        c.add_input(sig("a"));
        c.add_gate(Gate::new(GateKind::Buf, vec![sig("a")], sig("y")));
        c.add_gate(Gate::new(GateKind::Not, vec![sig("a")], sig("y")));
        c.add_output(sig("y"));
        let err = analyze(&c, Algebra::Kleene).unwrap_err();
        assert!(matches!(err, CircuitError::MultipleDrivers(ref s) if s == "y"));
    }

    #[test]
    fn rejects_duplicate_primary_inputs() {
        let mut c = Circuit::new("dup-input");
        c.add_input(sig("a"));
        c.add_input(sig("a"));
        c.add_gate(Gate::new(GateKind::Buf, vec![sig("a")], sig("y")));
        c.add_output(sig("y"));
        let err = analyze(&c, Algebra::Kleene).unwrap_err();
        assert!(matches!(err, CircuitError::DuplicateInput(ref s) if s == "a"));
    }

    #[test]
    fn evaluation_is_independent_of_gate_declaration_order() {
        let build = |reversed: bool| {
            let mut c = Circuit::new("chain");
            c.add_input(sig("a"));
            c.add_output(sig("out"));
            let gates = [
                Gate::new(GateKind::Not, vec![sig("a")], sig("n")),
                Gate::new(GateKind::Not, vec![sig("n")], sig("out")),
            ];
            if reversed {
                for gate in gates.into_iter().rev() {
                    c.add_gate(gate);
                }
            } else {
                for gate in gates {
                    c.add_gate(gate);
                }
            }
            c
        };

        for reversed in [false, true] {
            let c = build(reversed);
            let p = propagate(&c, &assignment(&[("a", Ternary::True)]), Algebra::Kleene).unwrap();
            assert_eq!(p.outputs["out"], Ternary::True, "reversed={reversed}");
        }

        let forward = analyze(&build(false), Algebra::Kleene).unwrap();
        let reversed = analyze(&build(true), Algebra::Kleene).unwrap();
        assert_eq!(forward.output_domains, reversed.output_domains);
        assert_eq!(
            forward.unknown_output_combinations,
            reversed.unknown_output_combinations
        );
        assert!(forward.output_domains["out"].contains(&Ternary::True));
    }

    #[test]
    fn rejects_undriven_primary_output() {
        let mut c = Circuit::new("ghost");
        c.add_input(sig("a"));
        c.add_output(sig("ghost"));
        let err = analyze(&c, Algebra::Kleene).unwrap_err();
        assert!(matches!(err, CircuitError::UndrivenSignal(ref s) if s == "ghost"));
    }

    #[test]
    fn pin_fault_does_not_leak_to_other_fanout() {
        let mut c = Circuit::new("fanout");
        c.add_input(sig("a"));
        c.add_input(sig("b"));
        c.add_gate(Gate::new(GateKind::And, vec![sig("a"), sig("b")], sig("n")));
        c.add_gate(Gate::new(GateKind::Buf, vec![sig("a")], sig("out")));
        c.add_output(sig("out"));

        let report = analyze(&c, Algebra::Kleene).unwrap();
        let dead_pin: Vec<&Fault> = report
            .faults
            .iter()
            .filter(|f| f.gate == "n" && f.input == "a")
            .collect();
        assert_eq!(dead_pin.len(), 2, "每个极性一条");
        assert!(
            dead_pin.iter().all(|f| !f.observable),
            "n 不通向主输出，两个极性都不可测"
        );
        assert!(
            !report
                .redundant_inputs
                .contains(&("n".to_string(), "a".to_string())),
            "该引脚会改变 n，只是观测不到，不是冗余"
        );
    }

    #[test]
    fn fault_polarities_are_reported_separately() {
        let mut c = Circuit::new("const-pin");
        c.add_input(sig("b"));
        c.add_gate(Gate::constant(Ternary::True, sig("one")));
        c.add_gate(Gate::new(
            GateKind::And,
            vec![sig("one"), sig("b")],
            sig("y"),
        ));
        c.add_output(sig("y"));

        let report = analyze(&c, Algebra::Kleene).unwrap();
        let sa0 = report
            .faults
            .iter()
            .find(|f| f.gate == "y" && f.input_index == 0 && f.stuck_at == Ternary::False)
            .expect("应有 SA-0");
        let sa1 = report
            .faults
            .iter()
            .find(|f| f.gate == "y" && f.input_index == 0 && f.stuck_at == Ternary::True)
            .expect("应有 SA-1");
        assert!(sa0.observable, "把常 1 拉到 0 会改变输出");
        assert!(!sa1.observable, "该引脚本来就是 1，SA-1 无变化");
    }

    #[test]
    fn untestable_reasons_classify_three_cases() {
        // 1) 输出到不了主输出
        let mut dead = Circuit::new("dead");
        dead.add_input(sig("a"));
        dead.add_input(sig("b"));
        dead.add_gate(Gate::new(GateKind::And, vec![sig("a"), sig("b")], sig("n")));
        dead.add_gate(Gate::new(GateKind::Buf, vec![sig("a")], sig("out")));
        dead.add_output(sig("out"));
        let r = analyze(&dead, Algebra::Kleene).unwrap();
        let unobserved = r
            .faults
            .iter()
            .find(|f| f.gate == "n" && f.input == "a" && f.stuck_at == Ternary::False)
            .unwrap();
        assert_eq!(unobserved.reason, Some(UntestableReason::Unobserved));

        // 2) 能到主输出但被常 1 的 OR 遮蔽
        let mut masked = Circuit::new("masked");
        masked.add_input(sig("a"));
        masked.add_input(sig("b"));
        masked.add_gate(Gate::constant(Ternary::True, sig("one")));
        masked.add_gate(Gate::new(GateKind::And, vec![sig("a"), sig("b")], sig("n")));
        masked.add_gate(Gate::new(
            GateKind::Or,
            vec![sig("n"), sig("one")],
            sig("y"),
        ));
        masked.add_output(sig("y"));
        let r = analyze(&masked, Algebra::Kleene).unwrap();
        let masked_fault = r
            .faults
            .iter()
            .find(|f| f.gate == "n" && f.input == "a" && f.stuck_at == Ternary::False)
            .unwrap();
        assert_eq!(masked_fault.reason, Some(UntestableReason::Masked));

        // 3) 固定引脚不改变门输出
        let mut redundant = Circuit::new("redundant");
        redundant.add_gate(Gate::constant(Ternary::True, sig("one")));
        redundant.add_gate(Gate::constant(Ternary::False, sig("zero")));
        redundant.add_gate(Gate::new(
            GateKind::And,
            vec![sig("one"), sig("zero")],
            sig("y"),
        ));
        redundant.add_output(sig("y"));
        let r = analyze(&redundant, Algebra::Kleene).unwrap();
        let count =
            |reason: UntestableReason| r.faults.iter().filter(|f| f.reason == Some(reason)).count();
        assert_eq!(
            count(UntestableReason::Redundant),
            2,
            "one 引脚两个极性都不改变输出"
        );
        assert_eq!(
            count(UntestableReason::Masked),
            1,
            "zero 引脚的 SA-0 与正常值相同"
        );
        assert_eq!(
            r.faults.iter().filter(|f| f.observable).count(),
            1,
            "zero 引脚的 SA-1 可测"
        );
    }
}
