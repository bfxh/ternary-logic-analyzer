//! 结构化 Verilog 子集：单比特组合逻辑（原语 + `assign` 简单表达式）。

use std::collections::HashSet;

use trit_core::ast::Expr;
use trit_core::parser::Parser;
use trit_core::value::Ternary;

use crate::circuit::{Circuit, Gate, GateKind, Signal};

/// Verilog 子集解析/生成错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerilogError {
    /// 语法或子集限制。
    #[error("{0}")]
    Message(String),
}

fn message(text: impl Into<String>) -> VerilogError {
    VerilogError::Message(text.into())
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '/' && index + 1 < chars.len() && chars[index + 1] == '/' {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
        } else if chars[index] == '/' && index + 1 < chars.len() && chars[index + 1] == '*' {
            index += 2;
            while index + 1 < chars.len() && !(chars[index] == '*' && chars[index + 1] == '/') {
                index += 1;
            }
            index = (index + 2).min(chars.len());
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

fn is_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
}

struct Builder<'a> {
    circuit: &'a mut Circuit,
    used: HashSet<String>,
    counter: usize,
}

impl Builder<'_> {
    fn fresh(&mut self) -> String {
        loop {
            let name = format!("__v{}", self.counter);
            self.counter += 1;
            if !self.used.contains(&name) {
                self.used.insert(name.clone());
                return name;
            }
        }
    }

    fn add_gate(&mut self, kind: GateKind, inputs: Vec<String>, output: &str) {
        self.used.insert(output.to_string());
        self.circuit.add_gate(Gate::new(
            kind,
            inputs.into_iter().map(Signal::new).collect(),
            Signal::new(output),
        ));
    }

    fn synth(&mut self, expr: &Expr, target: &str) -> Result<(), VerilogError> {
        match expr {
            Expr::Var(name) => {
                if name != target {
                    self.add_gate(GateKind::Buf, vec![name.clone()], target);
                }
            }
            Expr::Const(value) => {
                self.circuit
                    .add_gate(Gate::constant(*value, Signal::new(target)));
            }
            Expr::Not(inner) => {
                let source = self.synth_temp(inner)?;
                self.add_gate(GateKind::Not, vec![source], target);
            }
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => {
                let left = self.synth_temp(a)?;
                let right = self.synth_temp(b)?;
                let kind = match expr {
                    Expr::And(..) => GateKind::And,
                    Expr::Or(..) => GateKind::Or,
                    _ => GateKind::Xor,
                };
                self.add_gate(kind, vec![left, right], target);
            }
            Expr::Implies(..) | Expr::Equiv(..) => {
                return Err(message("assign 里暂不支持 => / <=>"));
            }
        }
        Ok(())
    }

    fn synth_temp(&mut self, expr: &Expr) -> Result<String, VerilogError> {
        match expr {
            Expr::Var(name) => Ok(name.clone()),
            Expr::Const(value) => {
                let name = self.fresh();
                self.circuit
                    .add_gate(Gate::constant(*value, Signal::new(name.clone())));
                Ok(name)
            }
            _ => {
                let name = self.fresh();
                self.synth(expr, &name)?;
                Ok(name)
            }
        }
    }
}

fn translate_assign(rhs: &str) -> Result<String, VerilogError> {
    if rhs.contains('^') {
        return Err(message(
            "暂不支持 ^ / xor：trit 的 ⊕ 是强加，不是 Verilog 布尔异或",
        ));
    }
    let mut text = rhs.replace("&&", "&").replace("||", "|");
    for (from, to) in [
        ("1'b0", "0"),
        ("1'b1", "1"),
        ("1'bx", "X"),
        ("1'bX", "X"),
        ("1'bz", "X"),
        ("1'bZ", "X"),
    ] {
        text = text.replace(from, to);
    }
    Ok(text)
}

fn split_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|part| part.trim().to_string())
        .collect()
}

fn declare_ports(
    circuit: &mut Circuit,
    direction: &str,
    list: &str,
    declared: &mut HashSet<String>,
) -> Result<(), VerilogError> {
    if list.contains('[') {
        return Err(message("暂不支持多比特端口"));
    }
    for name in split_list(list) {
        if !is_identifier(&name) {
            return Err(message(format!("非法端口名 {name:?}")));
        }
        if !declared.insert(name.clone()) {
            continue;
        }
        match direction {
            "input" => {
                circuit.add_input(Signal::new(name.clone()));
            }
            "output" => {
                circuit.add_output(Signal::new(name.clone()));
            }
            _ => {}
        }
    }
    Ok(())
}

/// 解析结构化 Verilog 子集。
pub fn parse_verilog(text: &str) -> Result<Circuit, VerilogError> {
    let cleaned = strip_comments(text);
    let Some(module_at) = cleaned.find("module") else {
        return Err(message("缺少 module 声明"));
    };
    let after_module = &cleaned[module_at + "module".len()..];
    let Some(header_end) = after_module.find(';') else {
        return Err(message("module 头缺少 ';'"));
    };
    let header = after_module[..header_end].trim();
    let Some(end_at) = after_module[header_end + 1..].find("endmodule") else {
        return Err(message("缺少 endmodule"));
    };
    let body = &after_module[header_end + 1..header_end + 1 + end_at];

    let (module_name, port_text) = match header.find('(') {
        Some(open) => {
            let close = header
                .rfind(')')
                .ok_or_else(|| message("module 端口列表缺少 ')'"))?;
            (
                header[..open].trim(),
                Some(header[open + 1..close].to_string()),
            )
        }
        None => (header, None),
    };
    if !is_identifier(module_name) {
        return Err(message(format!("非法模块名 {module_name:?}")));
    }
    let mut circuit = Circuit::new(module_name);
    let mut declared: HashSet<String> = HashSet::new();
    let mut wires: HashSet<String> = HashSet::new();

    if let Some(ports) = port_text.filter(|p| !p.trim().is_empty()) {
        let ansi = ports.contains("input") || ports.contains("output");
        if ansi {
            for part in ports.split(',') {
                let tokens: Vec<&str> = part.split_whitespace().collect();
                let Some(direction) = tokens.first().copied() else {
                    continue;
                };
                if direction != "input" && direction != "output" {
                    return Err(message(format!("未知端口方向 {direction:?}")));
                }
                let names: Vec<&str> = tokens
                    .iter()
                    .copied()
                    .filter(|token| *token != "input" && *token != "output" && *token != "wire")
                    .collect();
                declare_ports(&mut circuit, direction, &names.join(","), &mut declared)?;
            }
        } else {
            for name in split_list(&ports) {
                if !is_identifier(&name) {
                    return Err(message(format!("非法端口名 {name:?}")));
                }
                wires.insert(name);
            }
        }
    }

    for statement in body.split(';') {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        let first = statement.split_whitespace().next().unwrap_or("");
        match first {
            "input" | "output" => {
                for name in split_list(statement[first.len()..].trim()) {
                    if !is_identifier(&name) {
                        return Err(message(format!("非法端口名 {name:?}")));
                    }
                    if !declared.insert(name.clone()) {
                        continue;
                    }
                    if first == "input" {
                        circuit.add_input(Signal::new(name));
                    } else {
                        circuit.add_output(Signal::new(name));
                    }
                }
            }
            "wire" => {
                for name in split_list(statement["wire".len()..].trim()) {
                    if !is_identifier(&name) {
                        return Err(message(format!("非法 wire 名 {name:?}")));
                    }
                    wires.insert(name);
                }
            }
            "assign" => {
                let rest = statement["assign".len()..].trim();
                let Some((lhs, rhs)) = rest.split_once('=') else {
                    return Err(message("assign 缺少 '='"));
                };
                let lhs = lhs.trim();
                if !is_identifier(lhs) {
                    return Err(message(format!("非法 assign 目标 {lhs:?}")));
                }
                let expr = Parser::parse(&translate_assign(rhs.trim())?)
                    .map_err(|e| message(format!("assign 表达式解析失败：{e}")))?;
                let mut builder = Builder {
                    circuit: &mut circuit,
                    used: declared
                        .iter()
                        .cloned()
                        .chain(wires.iter().cloned())
                        .collect(),
                    counter: 0,
                };
                builder.synth(&expr, lhs)?;
            }
            "xor" | "xnor" => {
                return Err(message(
                    "暂不支持 xor/xnor：trit 的 ⊕ 是强加，不是 Verilog 布尔异或",
                ));
            }
            "and" | "or" | "nand" | "nor" | "not" | "buf" => {
                let normalized = statement
                    .replace('(', " ( ")
                    .replace(')', " ) ")
                    .replace(',', " , ");
                let tokens: Vec<&str> = normalized.split_whitespace().collect();
                let open = tokens
                    .iter()
                    .position(|token| *token == "(")
                    .ok_or_else(|| message(format!("{first} 缺少 '('")))?;
                let close = tokens
                    .iter()
                    .rposition(|token| *token == ")")
                    .ok_or_else(|| message(format!("{first} 缺少 ')'")))?;
                let args: Vec<&str> = tokens[open + 1..close]
                    .iter()
                    .copied()
                    .filter(|token| *token != ",")
                    .collect();
                if args.len() < 2 {
                    return Err(message(format!("{first} 至少需要输出和 1 个输入")));
                }
                let output = args[0];
                let inputs = &args[1..];
                let mut builder = Builder {
                    circuit: &mut circuit,
                    used: declared
                        .iter()
                        .cloned()
                        .chain(wires.iter().cloned())
                        .collect(),
                    counter: 0,
                };
                match first {
                    "not" | "buf" => {
                        if inputs.len() != 1 {
                            return Err(message(format!("{first} 只接受 1 个输入")));
                        }
                        let kind = if first == "not" {
                            GateKind::Not
                        } else {
                            GateKind::Buf
                        };
                        builder.add_gate(kind, vec![inputs[0].to_string()], output);
                    }
                    _ => {
                        let base = match first {
                            "and" => GateKind::And,
                            "or" => GateKind::Or,
                            "nand" => GateKind::Nand,
                            "nor" => GateKind::Nor,
                            "xnor" | "xor" => GateKind::Xor,
                            _ => unreachable!(),
                        };
                        let mut current = inputs[0].to_string();
                        for (index, input) in inputs[1..].iter().enumerate() {
                            let last = index + 2 == inputs.len();
                            let target = if last {
                                output.to_string()
                            } else {
                                builder.fresh()
                            };
                            builder.add_gate(
                                base,
                                vec![current.clone(), input.to_string()],
                                &target,
                            );
                            current = target;
                        }
                        if first == "xnor" {
                            builder.add_gate(GateKind::Not, vec![current], output);
                        }
                    }
                }
            }
            "always" | "reg" | "initial" => {
                return Err(message(format!("暂不支持时序语句 {first}")));
            }
            other => {
                return Err(message(format!("无法识别的语句 {other:?}")));
            }
        }
    }
    Ok(circuit)
}

fn gate_line(circuit_gate: &Gate, index: usize) -> String {
    let output = circuit_gate.output.name();
    let inputs: Vec<&str> = circuit_gate.inputs.iter().map(Signal::name).collect();
    match circuit_gate.kind {
        GateKind::Const => {
            let value = circuit_gate.const_value.unwrap_or(Ternary::False);
            format!(
                "  assign {output} = 1'b{};\n",
                value.as_char().to_ascii_lowercase()
            )
        }
        GateKind::Not | GateKind::Buf => format!(
            "  {} g{index} ({output}, {});\n",
            circuit_gate.kind.as_str().to_ascii_lowercase(),
            inputs[0]
        ),
        kind => format!(
            "  {} g{index} ({output}, {}, {});\n",
            kind.as_str().to_ascii_lowercase(),
            inputs[0],
            inputs[1]
        ),
    }
}

/// 生成结构化 Verilog（单比特）。
pub fn to_verilog(circuit: &Circuit) -> String {
    let module = if is_identifier(&circuit.name) {
        circuit.name.as_str()
    } else {
        "trit_top"
    };
    let mut ports: Vec<&str> = circuit.inputs.iter().map(Signal::name).collect();
    ports.extend(circuit.outputs.iter().map(Signal::name));
    let mut out = format!(
        "module {module} ({});\n",
        ports
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    for input in &circuit.inputs {
        out.push_str(&format!("  input {};\n", input.name()));
    }
    for output in &circuit.outputs {
        out.push_str(&format!("  output {};\n", output.name()));
    }
    let known: HashSet<&str> = ports.into_iter().collect();
    for gate in &circuit.gates {
        if !known.contains(gate.output.name()) {
            out.push_str(&format!("  wire {};\n", gate.output.name()));
        }
    }
    for (index, gate) in circuit.gates.iter().enumerate() {
        out.push_str(&gate_line(gate, index));
    }
    out.push_str("endmodule\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::analyze;
    use crate::netlist::parse_netlist;
    use trit_core::algebra::Algebra;

    #[test]
    fn parses_primitives_and_assign() {
        let text = "module m(input a, input b, output y, output z);\n\
                    wire n;\n\
                    and g1 (n, a, b);\n\
                    assign y = n;\n\
                    assign z = ~a | b;\n\
                    endmodule\n";
        let circuit = parse_verilog(text).unwrap();
        assert_eq!(circuit.input_count(), 2);
        assert_eq!(circuit.outputs.len(), 2);
        let report = analyze(&circuit, Algebra::Kleene).unwrap();
        assert!(report.gate_count >= 3);
    }

    #[test]
    fn round_trip_preserves_faults() {
        let circuit = parse_netlist(
            "input a\ninput b\nconst 1 one\nnot a na\ngate AND na b n\ngate OR n one y\noutput y\n",
        )
        .unwrap();
        let text = to_verilog(&circuit);
        assert!(text.contains("module netlist"), "{text}");
        let again = parse_verilog(&text).unwrap();
        let left = analyze(&circuit, Algebra::Kleene).unwrap();
        let right = analyze(&again, Algebra::Kleene).unwrap();
        assert_eq!(left.faults, right.faults);
        assert_eq!(left.output_domains, right.output_domains);
    }

    #[test]
    fn rejects_sequential() {
        let text = "module m(input a, output y);\nreg r;\nalways @(posedge a) r <= a;\nendmodule\n";
        assert!(parse_verilog(text).is_err());
    }
}
