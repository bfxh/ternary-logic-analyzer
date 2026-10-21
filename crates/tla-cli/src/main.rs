//! 三值逻辑分析器 —— 命令行入口。
//!
//! 子命令：
//!
//! - `eval`：在给定赋值下求值
//! - `table`：生成完整真值表
//! - `equiv`：判定两个表达式是否逻辑等价
//! - `simplify`：化简表达式
//! - `circuit`：分析三值门级网表
//! - `repl`：交互式会话

mod output;

use std::io::{BufRead, Write};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tla_core::algebra::Algebra;
use tla_core::ast::Expr;
use tla_core::circuit::{Circuit, Gate, GateKind, Signal};
use tla_core::eval::{Assignment, decode_assignment, equivalent, eval, truth_table};
use tla_core::parser::Parser as ExprParser;
use tla_core::simplify::{SimplifyOptions, simplify_with};
use tla_core::symbolic::equivalent_symbolic;
use tla_core::value::Ternary;

use crate::output::Format;

/// 三值逻辑分析器。
#[derive(Debug, Parser)]
#[command(
    name = "tla",
    version,
    about = "三值逻辑分析器：Kleene / Łukasiewicz / Bochvar 三套语义的求值、真值表、等价判定与化简",
    long_about = None
)]
struct Cli {
    /// 使用的三值语义：`kleene`（默认）、`lukasiewicz`、`bochvar`。
    #[arg(
        long,
        short = 'a',
        global = true,
        default_value = "kleene",
        value_name = "ALG"
    )]
    algebra: String,

    /// 子命令。
    #[command(subcommand)]
    command: Commands,
}

/// CLI 子命令。
#[derive(Debug, Subcommand)]
enum Commands {
    /// 在指定赋值下求值表达式。
    Eval {
        /// 待求值的表达式，例如 `a & b | !c`。
        expression: String,
        /// 变量赋值，形如 `a=1,b=X`。
        #[arg(long, short = 's', value_name = "ASSIG", default_value = "")]
        assign: String,
    },
    /// 生成表达式的完整真值表。
    Table {
        /// 待求值的表达式。
        expression: String,
        /// 输出格式。
        #[arg(long, short = 'f', value_name = "FORMAT", default_value = "table")]
        format: Format,
    },
    /// 判定两个表达式是否逻辑等价。
    Equiv {
        /// 第一个表达式。
        left: String,
        /// 第二个表达式。
        right: String,
        /// 输出格式。
        #[arg(long, short = 'f', value_name = "FORMAT", default_value = "table")]
        format: Format,
        /// 用 SAT 符号化判定（dual-rail），不做 3^n 穷举。
        #[arg(long)]
        symbolic: bool,
        /// 穷举判定的变量数上限；超过则报错并提示 `--symbolic`。
        #[arg(long, default_value_t = 18)]
        max_vars: usize,
    },
    /// 化简表达式。
    Simplify {
        /// 待化简的表达式。
        expression: String,
        /// 输出格式。
        #[arg(long, short = 'f', value_name = "FORMAT", default_value = "table")]
        format: Format,
        /// 是否把 `a ⇒ b` 展开为 `¬a ∨ b`（仅 Kleene 语义合法）。
        #[arg(long)]
        expand_implies: bool,
    },
    /// 分析三值门级网表。
    Circuit {
        /// 网表文件路径。
        path: std::path::PathBuf,
        /// 输出格式。
        #[arg(long, short = 'f', value_name = "FORMAT", default_value = "table")]
        format: Format,
    },
    /// 交互式会话：逐行输入表达式，直接看到真值表。
    Repl {
        /// 启动时使用的语义。
        #[arg(long, value_name = "ALG")]
        algebra: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Eval { expression, assign } => cmd_eval(&expression, &assign, &cli.algebra),
        Commands::Table { expression, format } => cmd_table(&expression, format, &cli.algebra),
        Commands::Equiv {
            left,
            right,
            format,
            symbolic,
            max_vars,
        } => cmd_equiv(&left, &right, format, &cli.algebra, symbolic, max_vars),
        Commands::Simplify {
            expression,
            format,
            expand_implies,
        } => cmd_simplify(&expression, format, expand_implies, &cli.algebra),
        Commands::Circuit { path, format } => cmd_circuit(&path, format, &cli.algebra),
        Commands::Repl { algebra } => {
            let alg_name = algebra.unwrap_or(cli.algebra);
            cmd_repl(&alg_name)
        }
    }
}

/// 解析语义名称。
fn resolve_algebra(name: &str) -> Result<Algebra> {
    Algebra::parse(name)
        .with_context(|| format!("未知的三值语义 {name:?}，可选：kleene / lukasiewicz / bochvar"))
}

/// 解析表达式。
fn parse_expr(src: &str) -> Result<Expr> {
    ExprParser::parse(src).with_context(|| format!("表达式解析失败：{src}"))
}

/// 解析赋值串 `a=1,b=X`。
fn parse_assignment(src: &str) -> Result<Assignment> {
    let mut asg = Assignment::new();
    for part in src.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((name, value)) = part.split_once('=') else {
            bail!("赋值格式应为 name=value，收到 {part:?}");
        };
        let name = name.trim();
        let value = value.trim();
        let v = match value {
            "0" | "false" | "False" | "f" => Ternary::False,
            "1" | "true" | "True" | "t" => Ternary::True,
            "X" | "x" | "?" | "unknown" | "Unknown" => Ternary::Unknown,
            other => bail!("未知取值 {other:?}，只能是 0 / 1 / X"),
        };
        asg.set(name.to_string(), v);
    }
    Ok(asg)
}

/// `eval` 子命令。
fn cmd_eval(expression: &str, assign: &str, alg_name: &str) -> Result<()> {
    let alg = resolve_algebra(alg_name)?;
    let expr = parse_expr(expression)?;
    let asg = parse_assignment(assign)?;
    let value = eval(&expr, &asg, alg);
    println!("{value}");
    Ok(())
}

/// `table` 子命令。
fn cmd_table(expression: &str, format: Format, alg_name: &str) -> Result<()> {
    let alg = resolve_algebra(alg_name)?;
    let expr = parse_expr(expression)?;
    let tt = truth_table(&expr, alg);
    print!(
        "{}",
        format.render_truth_table(&tt, &format!("f = {expression}  [{alg}]"))
    );
    Ok(())
}

/// `equiv` 子命令。
fn cmd_equiv(
    left: &str,
    right: &str,
    format: Format,
    alg_name: &str,
    symbolic: bool,
    max_vars: usize,
) -> Result<()> {
    let alg = resolve_algebra(alg_name)?;
    let a = parse_expr(left)?;
    let b = parse_expr(right)?;
    let mut vars = a.variables();
    for v in b.variables() {
        if !vars.contains(&v) {
            vars.push(v);
        }
    }
    let (result, method) = if symbolic {
        (equivalent_symbolic(&a, &b, alg)?, "sat")
    } else {
        if vars.len() > max_vars {
            bail!(
                "变量数 {} 超过穷举上限 {}；用 --symbolic 走 SAT 判定",
                vars.len(),
                max_vars
            );
        }
        (equivalent(&a, &b, alg), "exhaustive")
    };
    match format {
        Format::Json => {
            let mut obj = serde_json::json!({
                "equivalent": result.equivalent,
                "algebra": alg.as_str(),
                "variable_count": result.variable_count,
                "method": method,
            });
            if let Some(ce) = result.counterexample {
                let asg = decode_assignment(ce, &vars);
                let mut pairs = serde_json::Map::new();
                for name in &vars {
                    pairs.insert(
                        name.clone(),
                        serde_json::Value::String(
                            asg.get(name)
                                .unwrap_or(Ternary::Unknown)
                                .as_char()
                                .to_string(),
                        ),
                    );
                }
                obj["counterexample"] = serde_json::Value::Object(pairs);
                obj["counterexample_index"] = serde_json::json!(ce);
            }
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
        _ => {
            if result.equivalent {
                println!("等价：{left} ≡ {right}  [{alg}]");
            } else {
                let ce = result.counterexample.unwrap_or(0);
                let asg = decode_assignment(ce, &vars);
                let detail = vars
                    .iter()
                    .map(|v| format!("{v}={}", asg.get(v).unwrap_or(Ternary::Unknown).as_char()))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("不等价：{left} ≢ {right}  [{alg}]");
                println!("反例：{detail}（赋值编号 {ce}）");
            }
        }
    }
    if result.equivalent {
        Ok(())
    } else {
        std::process::exit(2)
    }
}

/// `simplify` 子命令。
fn cmd_simplify(
    expression: &str,
    format: Format,
    expand_implies: bool,
    alg_name: &str,
) -> Result<()> {
    let alg = resolve_algebra(alg_name)?;
    let expr = parse_expr(expression)?;
    let opts = SimplifyOptions {
        expand_implies,
        ..SimplifyOptions::default()
    };
    let simplified = simplify_with(&expr, alg, opts);
    match format {
        Format::Json => {
            let obj = serde_json::json!({
                "original": expr.to_infix(),
                "simplified": simplified.to_infix(),
                "algebra": alg.as_str(),
                "equivalent": equivalent(&expr, &simplified, alg).equivalent,
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
        _ => {
            println!("原式：{}", expr.to_infix());
            println!("化简：{}", simplified.to_infix());
            println!("语义：{alg}");
            let eq = equivalent(&expr, &simplified, alg);
            println!("语义等价：{}", if eq.equivalent { "是" } else { "否" });
        }
    }
    Ok(())
}

/// `circuit` 子命令：读取网表文件并分析。
///
/// 网表文件格式（每行一条，空行与 `#` 注释忽略）：
///
/// ```text
/// input a
/// input b
/// output out
/// gate AND a b out
/// ```
fn cmd_circuit(path: &std::path::Path, format: Format, alg_name: &str) -> Result<()> {
    let alg = resolve_algebra(alg_name)?;
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("无法读取网表文件 {}", path.display()))?;
    let circuit =
        parse_netlist(&text).with_context(|| format!("网表解析失败：{}", path.display()))?;
    let report = tla_core::circuit::analyze(&circuit, alg)
        .with_context(|| format!("网表分析失败：{}", path.display()))?;

    match format {
        Format::Json => {
            let obj = serde_json::json!({
                "circuit": report.circuit,
                "algebra": report.algebra.as_str(),
                "input_count": report.input_count,
                "output_count": report.output_count,
                "gate_count": report.gate_count,
                "unknown_output_combinations": report.unknown_output_combinations,
                "redundant_inputs": report.redundant_inputs
                    .iter()
                    .map(|(g, s)| serde_json::json!({"gate": g, "input": s}))
                    .collect::<Vec<_>>(),
                "untestable_faults": report.untestable_faults
                    .iter()
                    .map(|(g, s)| serde_json::json!({"gate": g, "input": s}))
                    .collect::<Vec<_>>(),
                "output_domains": report.output_domains
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            serde_json::Value::Array(
                                v.iter().map(|t| serde_json::Value::String(t.as_char().to_string())).collect(),
                            ),
                        )
                    })
                    .collect::<serde_json::Map<String, serde_json::Value>>(),
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
        _ => {
            println!("电路：{}  [{}]", report.circuit, report.algebra);
            println!(
                "主输入 {} 个，主输出 {} 个，门 {} 个",
                report.input_count, report.output_count, report.gate_count
            );
            println!(
                "输出含未知（X）的组合数：{} / {}",
                report.unknown_output_combinations,
                circuit.combination_count()
            );
            println!("\n输出值域：");
            for (name, domain) in &report.output_domains {
                let vals: Vec<String> = domain.iter().map(|v| v.as_char().to_string()).collect();
                println!("  {name} = {{{}}}", vals.join(", "));
            }
            if !report.redundant_inputs.is_empty() {
                println!("\n冗余输入（可删除）：");
                for (gate, sig) in &report.redundant_inputs {
                    println!("  门 {gate} 的输入 {sig}");
                }
            }
            if !report.untestable_faults.is_empty() {
                println!("\n不可测故障（stuck-at 无法观测）：");
                for (gate, sig) in &report.untestable_faults {
                    println!("  门 {gate} 的输入 {sig}");
                }
            }
        }
    }
    Ok(())
}

/// 解析网表文本。
fn parse_netlist(text: &str) -> Result<Circuit> {
    let mut circuit = Circuit::new("netlist");
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        let ctx = format!("第 {} 行", lineno + 1);
        match parts[0] {
            "input" => {
                if parts.len() != 2 {
                    bail!("{ctx}: input 需要 1 个参数");
                }
                circuit.add_input(Signal::new(parts[1]));
            }
            "output" => {
                if parts.len() != 2 {
                    bail!("{ctx}: output 需要 1 个参数");
                }
                circuit.add_output(Signal::new(parts[1]));
            }
            "gate" => {
                if parts.len() != 5 {
                    bail!("{ctx}: gate 语法为 `gate <类型> <输入1> <输入2> <输出>`");
                }
                let kind = GateKind::parse(parts[1])
                    .with_context(|| format!("{ctx}: 未知门类型 {:?}", parts[1]))?;
                let inputs = vec![Signal::new(parts[2]), Signal::new(parts[3])];
                circuit.add_gate(Gate::new(kind, inputs, Signal::new(parts[4])));
            }
            "not" => {
                if parts.len() != 3 {
                    bail!("{ctx}: not 语法为 `not <输入> <输出>`");
                }
                circuit.add_gate(Gate::new(
                    GateKind::Not,
                    vec![Signal::new(parts[1])],
                    Signal::new(parts[2]),
                ));
            }
            other => bail!("{ctx}: 未知指令 {other:?}"),
        }
    }
    if circuit.inputs.is_empty() {
        bail!("网表没有主输入");
    }
    if circuit.outputs.is_empty() {
        bail!("网表没有主输出");
    }
    Ok(circuit)
}

/// `repl` 子命令：交互式会话。
fn cmd_repl(alg_name: &str) -> Result<()> {
    let alg = resolve_algebra(alg_name)?;
    println!("三值逻辑分析器 REPL  [{alg}]");
    println!("输入表达式查看真值表；`:help` 看命令，`:quit` 退出。");

    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        print!("tla> ");
        std::io::stdout().flush().ok();
        line.clear();
        let n = stdin.lock().read_line(&mut line)?;
        if n == 0 {
            println!();
            break;
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        match input {
            ":quit" | ":q" | ":exit" => break,
            ":help" | ":h" => {
                println!("  <表达式>     显示真值表");
                println!("  :algebra <名> 切换语义（kleene/lukasiewicz/bochvar）");
                println!("  :quit       退出");
            }
            _ => {
                if let Some(rest) = input.strip_prefix(":algebra") {
                    match resolve_algebra(rest.trim()) {
                        Ok(a) => {
                            println!("已切换到 {a}");
                        }
                        Err(e) => println!("{e:#}"),
                    }
                    continue;
                }
                match ExprParser::parse(input) {
                    Ok(expr) => {
                        let tt = truth_table(&expr, alg);
                        print!("{}", Format::Table.render_truth_table(&tt, input));
                    }
                    Err(e) => println!("解析错误：{e}"),
                }
            }
        }
    }
    Ok(())
}
