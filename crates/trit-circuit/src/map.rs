//! 技术映射：把组合网表降到 NAND（+ 常量）门。

use std::collections::HashSet;

use crate::circuit::{Circuit, Gate, GateKind, Signal};

fn nand(circuit: &mut Circuit, a: &str, b: &str, out: &str) {
    circuit.add_gate(Gate::new(
        GateKind::Nand,
        vec![Signal::new(a), Signal::new(b)],
        Signal::new(out),
    ));
}

/// 映射失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MapError {
    /// 该门无法用 NAND + 常量表达（例如三值强加需要 X 检测单元）。
    #[error("门 {0:?} 无法映射到 NAND-only")]
    Unsupported(GateKind),
}

/// 把电路映射成只用 NAND 与常量门的等价电路。
pub fn to_nand_only(circuit: &Circuit) -> Result<Circuit, MapError> {
    let mut mapped = Circuit::new(format!("{}_nand", circuit.name));
    let mut used: HashSet<String> = HashSet::new();
    for input in &circuit.inputs {
        used.insert(input.name().to_string());
        mapped.add_input(Signal::new(input.name()));
    }
    let mut counter = 0usize;
    let mut fresh = || {
        loop {
            let name = format!("__nand{counter}");
            counter += 1;
            if !used.contains(&name) {
                used.insert(name.clone());
                return name;
            }
        }
    };

    for gate in &circuit.gates {
        let output = gate.output.name();
        let a = gate.inputs.first().map(Signal::name).unwrap_or("");
        let b = gate.inputs.get(1).map(Signal::name).unwrap_or(a);
        match gate.kind {
            GateKind::Const => {
                let value = gate.const_value.unwrap_or(trit_core::value::Ternary::False);
                mapped.add_gate(Gate::constant(value, Signal::new(output)));
            }
            GateKind::Buf => {
                let t = fresh();
                nand(&mut mapped, a, a, &t);
                nand(&mut mapped, &t, &t, output);
            }
            GateKind::Not => nand(&mut mapped, a, a, output),
            GateKind::Nand => nand(&mut mapped, a, b, output),
            GateKind::And => {
                let t = fresh();
                nand(&mut mapped, a, b, &t);
                nand(&mut mapped, &t, &t, output);
            }
            GateKind::Or => {
                let na = fresh();
                let nb = fresh();
                nand(&mut mapped, a, a, &na);
                nand(&mut mapped, b, b, &nb);
                nand(&mut mapped, &na, &nb, output);
            }
            GateKind::Nor => {
                let na = fresh();
                let nb = fresh();
                let n = fresh();
                nand(&mut mapped, a, a, &na);
                nand(&mut mapped, b, b, &nb);
                nand(&mut mapped, &na, &nb, &n);
                nand(&mut mapped, &n, &n, output);
            }
            // `Xor` 是三值强加（X 主导），没有 X 检测单元就无法用 NAND 表达。
            GateKind::Xor => return Err(MapError::Unsupported(GateKind::Xor)),
            GateKind::XorClassic => {
                let n1 = fresh();
                let n2 = fresh();
                let n3 = fresh();
                nand(&mut mapped, a, b, &n1);
                nand(&mut mapped, a, &n1, &n2);
                nand(&mut mapped, b, &n1, &n3);
                nand(&mut mapped, &n2, &n3, output);
            }
        }
    }
    for output in &circuit.outputs {
        mapped.add_output(Signal::new(output.name()));
    }
    Ok(mapped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::{Gate, GateKind, Signal, propagate};
    use trit_core::algebra::Algebra;
    use trit_core::value::Ternary;

    fn check(kind: GateKind, inputs: &[&str], output: &str) {
        let mut circuit = Circuit::new("orig");
        for input in inputs {
            circuit.add_input(Signal::new(*input));
        }
        circuit.add_gate(Gate::new(
            kind,
            inputs.iter().map(|name| Signal::new(*name)).collect(),
            Signal::new(output),
        ));
        circuit.add_output(Signal::new(output));
        let mapped = to_nand_only(&circuit).unwrap();

        let total = 3u64.pow(inputs.len() as u32);
        for combo in 0..total {
            let mut assignment = std::collections::BTreeMap::new();
            let mut rest = combo;
            for input in inputs {
                let value = match rest % 3 {
                    0 => Ternary::False,
                    1 => Ternary::True,
                    _ => Ternary::Unknown,
                };
                rest /= 3;
                assignment.insert(input.to_string(), value);
            }
            let left = propagate(&circuit, &assignment, Algebra::Kleene).unwrap();
            let right = propagate(&mapped, &assignment, Algebra::Kleene).unwrap();
            assert_eq!(
                left.signals.get(output),
                right.signals.get(output),
                "{kind:?} {inputs:?} combo {combo}"
            );
        }
    }

    #[test]
    fn maps_all_gate_kinds_equivalently() {
        check(GateKind::And, &["a", "b"], "y");
        check(GateKind::Or, &["a", "b"], "y");
        check(GateKind::XorClassic, &["a", "b"], "y");
        check(GateKind::Nand, &["a", "b"], "y");
        check(GateKind::Nor, &["a", "b"], "y");
        check(GateKind::Not, &["a"], "y");
        check(GateKind::Buf, &["a"], "y");
    }

    #[test]
    fn uses_only_nand_and_const() {
        let mut circuit = Circuit::new("mix");
        circuit.add_input(Signal::new("a"));
        circuit.add_input(Signal::new("b"));
        circuit.add_gate(Gate::new(
            GateKind::Or,
            vec![Signal::new("a"), Signal::new("b")],
            Signal::new("y"),
        ));
        circuit.add_output(Signal::new("y"));
        let mapped = to_nand_only(&circuit).unwrap();
        assert!(
            mapped
                .gates
                .iter()
                .all(|gate| matches!(gate.kind, GateKind::Nand | GateKind::Const))
        );
    }
}
