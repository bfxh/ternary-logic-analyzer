//! 三值逻辑分析器 —— 命令行入口。
//!
//! 子命令：
//!
//! - `eval`：在给定赋值下求值
//! - `table`：生成完整真值表
//! - `equiv`：判定两个表达式是否逻辑等价
//! - `quantify`：对三值变量做存在/全称量化
//! - `simplify`：化简表达式
//! - `circuit`：分析三值门级网表（`--assign` 对一条含 X 的输入向量做单遍传播）
//! - `belnap`：Belnap 四值（`T`/`F`/`B`/`N`）的 opt-in 求值、真值表与等价
//! - `repl`：交互式会话

mod output;

use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, Write};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use trit_bdd::{
    BddLimits, BddRequest, Quantifier, VarOrder, equivalent_bdd_best_with_limits,
    equivalent_bdd_with, order_variables, quantify_bdd_best_with_limits, quantify_bdd_with,
};
use trit_circuit::netlist::parse_netlist;
use trit_core::algebra::Algebra;
use trit_core::ast::Expr;
use trit_core::limits::Limits;
use trit_core::parser::Parser as ExprParser;
use trit_core::value::Ternary;
use trit_eval::eval::{Assignment, equivalent, equivalent_within, eval, truth_table_within};
use trit_eval::simplify::{SimplifyOptions, simplify_with};
use trit_policy::{Engine, Policy};
use trit_sat::symbolic::{
    EncodingSize, encoding_size, equivalent_symbolic_with, minimal_counterexample,
};

use crate::output::Format;

/// 三值逻辑分析器。
#[derive(Debug, Parser)]
#[command(
    name = "trit",
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
        /// 用 BDD（canonical）判定；与 `--symbolic` 互斥。
        #[arg(long)]
        bdd: bool,
        /// BDD 变量排序：appearance（默认）/ reverse / frequency / frequency-asc / best。
        #[arg(long, value_name = "ORDER", default_value = "appearance")]
        bdd_order: String,
        /// BDD 单图节点预算（0 表示不设上限）。
        #[arg(long, value_name = "N", default_value_t = 1_000_000)]
        bdd_max_nodes: usize,
        /// 穷举判定的变量数上限；超过则报错并提示 `--symbolic`。
        #[arg(long, default_value_t = 18)]
        max_vars: usize,
        /// 把列出的变量当作已知二值（逗号分隔）；其子图走经典 CNF。
        #[arg(long, value_name = "VARS")]
        binary: Option<String>,
        /// 把所有变量当作已知二值。
        #[arg(long)]
        binary_all: bool,
        /// 打印 SAT 编码规模（仅 `--symbolic`）。
        #[arg(long)]
        stats: bool,
        /// 输出最小反例：只保留区分两边所必需的变量（仅 `--symbolic`）。
        #[arg(long)]
        minimize: bool,
    },
    /// 对三值变量做存在/全称量化（BDD）。
    Quantify {
        /// 待量化的表达式，例如 `a & b`。
        expression: String,
        /// 要量化的变量（逗号分隔）；省略则量化全部变量。
        #[arg(long, value_name = "VARS")]
        vars: Option<String>,
        /// 量化方向：`exists`（默认）或 `forall`。
        #[arg(long, default_value = "exists", value_name = "KIND")]
        kind: String,
        /// 把列出的变量当作已知二值（逗号分隔），其量化域为 {0,1}。
        #[arg(long, value_name = "VARS")]
        binary: Option<String>,
        /// 把所有变量当作已知二值。
        #[arg(long)]
        binary_all: bool,
        /// BDD 变量排序：appearance（默认）/ reverse / frequency / frequency-asc / best。
        #[arg(long, value_name = "ORDER", default_value = "appearance")]
        bdd_order: String,
        /// BDD 单图节点预算（0 表示不设上限）。
        #[arg(long, value_name = "N", default_value_t = 1_000_000)]
        bdd_max_nodes: usize,
        /// JSON 输出。
        #[arg(long)]
        json: bool,
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
        /// 单向量传播：`a=1,b=X`（给全所有主输入）。
        #[arg(long, value_name = "ASSIGN")]
        assign: Option<String>,
        /// 输出格式。
        #[arg(long, short = 'f', value_name = "FORMAT", default_value = "table")]
        format: Format,
    },
    /// 生成 stuck-at 测试向量并报告覆盖率。
    Atpg {
        /// 网表文件路径。
        path: std::path::PathBuf,
        /// 输出格式。
        #[arg(long, short = 'f', value_name = "FORMAT", default_value = "table")]
        format: Format,
    },
    /// 网表格式转换：text <-> json。
    Netlist {
        /// 输入文件。
        path: std::path::PathBuf,
        /// 输入格式：`text`（默认）或 `json`。
        #[arg(long, default_value = "text")]
        from: String,
        /// 输出格式：`json`（默认）或 `text`。
        #[arg(long, default_value = "json")]
        to: String,
    },
    /// 验证三值决策表策略文件。
    Verify {
        /// 策略 JSON 路径。
        path: std::path::PathBuf,
        /// 引擎：sat（默认）或 bdd。
        #[arg(long, default_value = "sat")]
        engine: String,
        /// BDD 变量排序（仅 --engine bdd）：appearance（默认）/ reverse / frequency / frequency-asc。
        #[arg(long, value_name = "ORDER", default_value = "appearance")]
        bdd_order: String,
        /// BDD 单图节点预算（仅 --engine bdd；0 表示不设上限）。
        #[arg(long, value_name = "N", default_value_t = 1_000_000)]
        bdd_max_nodes: usize,
        /// JSON 输出。
        #[arg(long)]
        json: bool,
    },
    /// Belnap 四值（opt-in）：T / F / B / N。
    Belnap {
        /// 子命令。
        #[command(subcommand)]
        cmd: BelnapCmd,
    },
    /// 交互式会话：逐行输入表达式，直接看到真值表。
    Repl {
        /// 启动时使用的语义。
        #[arg(long, value_name = "ALG")]
        algebra: Option<String>,
    },
}

/// `belnap` 的子命令。
#[derive(Debug, Subcommand)]
enum BelnapCmd {
    /// 在给定赋值下求值；未列出的变量按 N。
    Eval {
        /// 表达式。
        expression: String,
        /// 赋值：`a=T,b=B`。
        #[arg(long, short = 's', default_value = "")]
        assign: String,
    },
    /// 打印 4^n 真值表（最多 6 个变量）。
    Table {
        /// 表达式。
        expression: String,
    },
    /// 在 4^n 上判定两个表达式是否逐点相等；不等价退出码 2。
    Equiv {
        /// 左表达式。
        left: String,
        /// 右表达式。
        right: String,
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
            bdd,
            bdd_order,
            bdd_max_nodes,
            max_vars,
            binary,
            binary_all,
            stats,
            minimize,
        } => cmd_equiv(
            &left,
            &right,
            format,
            &cli.algebra,
            EquivOptions {
                symbolic,
                bdd,
                bdd_order,
                bdd_max_nodes,
                max_vars,
                binary,
                binary_all,
                stats,
                minimize,
            },
        ),
        Commands::Quantify {
            expression,
            vars,
            kind,
            binary,
            binary_all,
            bdd_order,
            bdd_max_nodes,
            json,
        } => cmd_quantify(
            &expression,
            &cli.algebra,
            QuantifyOptions {
                vars,
                kind: &kind,
                binary,
                binary_all,
                bdd_order,
                bdd_max_nodes,
                json,
            },
        ),
        Commands::Simplify {
            expression,
            format,
            expand_implies,
        } => cmd_simplify(&expression, format, expand_implies, &cli.algebra),
        Commands::Circuit {
            path,
            assign,
            format,
        } => cmd_circuit(&path, format, &cli.algebra, assign.as_deref()),
        Commands::Atpg { path, format } => cmd_atpg(&path, format, &cli.algebra),
        Commands::Netlist { path, from, to } => cmd_netlist(&path, &from, &to),
        Commands::Verify {
            path,
            engine,
            bdd_order,
            bdd_max_nodes,
            json,
        } => cmd_verify(&path, &engine, &bdd_order, bdd_max_nodes, json),
        Commands::Belnap { cmd } => cmd_belnap(cmd),
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
    let limits = Limits {
        max_vars: 10,
        ..Limits::default()
    };
    let tt = truth_table_within(&expr, alg, limits)
        .with_context(|| "真值表超出预算（最多 10 个变量，3^11 = 177147 行）")?;
    print!(
        "{}",
        format.render_truth_table(&tt, &format!("f = {expression}  [{alg}]"))
    );
    Ok(())
}

/// `equiv` 子命令。
struct EquivOptions {
    symbolic: bool,
    bdd: bool,
    bdd_order: String,
    bdd_max_nodes: usize,
    max_vars: usize,
    binary: Option<String>,
    binary_all: bool,
    stats: bool,
    minimize: bool,
}

fn cmd_equiv(
    left: &str,
    right: &str,
    format: Format,
    alg_name: &str,
    opts: EquivOptions,
) -> Result<()> {
    let EquivOptions {
        symbolic,
        bdd,
        bdd_order,
        bdd_max_nodes,
        max_vars,
        binary: binary_list,
        binary_all,
        stats,
        minimize,
    } = opts;
    let alg = resolve_algebra(alg_name)?;
    let a = parse_expr(left)?;
    let b = parse_expr(right)?;
    let mut vars = a.variables();
    for v in b.variables() {
        if !vars.contains(&v) {
            vars.push(v);
        }
    }
    let mut binary: HashSet<String> = HashSet::new();
    if binary_all {
        binary.extend(vars.iter().cloned());
    }
    if let Some(list) = binary_list {
        for name in list.split(',') {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            if !vars.iter().any(|v| v == name) {
                bail!("--binary 变量 {name} 不在表达式里");
            }
            binary.insert(name.to_string());
        }
    }
    if bdd && symbolic {
        bail!("--bdd 与 --symbolic 只能选一个");
    }
    if !binary.is_empty() && !symbolic && !bdd {
        bail!("--binary / --binary-all 只用于 --symbolic 或 --bdd");
    }
    if minimize && !symbolic {
        bail!("--minimize 只用于 --symbolic");
    }
    if minimize {
        let witness = minimal_counterexample(&a, &b, alg, &binary)?;
        match format {
            Format::Json => {
                let mut obj = serde_json::json!({
                    "algebra": alg.as_str(),
                    "method": "sat-min",
                    "equivalent": witness.is_none(),
                });
                if let Some(asg) = &witness {
                    let mut pairs = serde_json::Map::new();
                    for name in asg.variables() {
                        pairs.insert(
                            name.clone(),
                            serde_json::Value::String(
                                asg.get(&name)
                                    .unwrap_or(Ternary::Unknown)
                                    .as_char()
                                    .to_string(),
                            ),
                        );
                    }
                    obj["counterexample_minimal"] = serde_json::Value::Object(pairs);
                }
                println!("{}", serde_json::to_string_pretty(&obj)?);
            }
            _ => {
                if let Some(asg) = &witness {
                    let detail = asg
                        .variables()
                        .iter()
                        .map(|name| {
                            format!(
                                "{name}={}",
                                asg.get(name).unwrap_or(Ternary::Unknown).as_char()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("不等价：{left} ≢ {right}  [{alg} / sat-min]");
                    println!("最小反例：{detail}");
                } else {
                    println!("等价：{left} ≡ {right}  [{alg} / sat-min]");
                }
            }
        }
        return if witness.is_none() {
            Ok(())
        } else {
            std::process::exit(2)
        };
    }
    let stats_value: Option<EncodingSize> = if symbolic && stats {
        Some(encoding_size(&a, &b, alg, &binary)?)
    } else {
        None
    };
    let limits = if bdd_max_nodes == 0 {
        BddLimits::UNLIMITED
    } else {
        BddLimits {
            max_nodes: bdd_max_nodes,
        }
    };
    let mut bdd_order_used = None;
    let (result, method, bdd_nodes) = if bdd {
        let verdict = match bdd_order.as_str() {
            "best" => {
                let (verdict, order) =
                    equivalent_bdd_best_with_limits(&a, &b, alg, &binary, limits)?;
                bdd_order_used = Some(order.as_str());
                verdict
            }
            name => {
                let order = VarOrder::parse(name)
                    .ok_or_else(|| anyhow::anyhow!("未知 BDD 排序：{name}"))?;
                bdd_order_used = Some(order.as_str());
                let vars = (order != VarOrder::Appearance).then(|| order_variables(&a, &b, order));
                let request = BddRequest {
                    order: vars.as_deref(),
                    limits,
                };
                equivalent_bdd_with(&a, &b, alg, &binary, &request)?
            }
        };
        (verdict.result, "bdd", Some(verdict.nodes))
    } else if symbolic {
        let method = if binary.is_empty() {
            "sat"
        } else {
            "sat-binary"
        };
        (
            equivalent_symbolic_with(&a, &b, alg, &binary)?,
            method,
            None,
        )
    } else {
        if vars.len() > max_vars {
            bail!(
                "变量数 {} 超过穷举上限 {}；用 --symbolic 或 --bdd 走符号化判定",
                vars.len(),
                max_vars
            );
        }
        (equivalent(&a, &b, alg), "exhaustive", None)
    };
    match format {
        Format::Json => {
            let mut obj = serde_json::json!({
                "equivalent": result.equivalent,
                "algebra": alg.as_str(),
                "variable_count": result.variable_count,
                "method": method,
            });
            if let Some(size) = &stats_value {
                obj["encoding"] = serde_json::json!({
                    "variables": size.variables,
                    "clauses": size.clauses,
                });
            }
            if let Some(nodes) = bdd_nodes.filter(|_| stats) {
                obj["bdd_nodes"] = serde_json::json!(nodes);
            }
            if let Some(order) = bdd_order_used {
                obj["bdd_order"] = serde_json::json!(order);
            }
            if let Some(asg) = &result.counterexample_assignment {
                let mut pairs = serde_json::Map::new();
                for name in asg.variables() {
                    pairs.insert(
                        name.clone(),
                        serde_json::Value::String(
                            asg.get(&name)
                                .unwrap_or(Ternary::Unknown)
                                .as_char()
                                .to_string(),
                        ),
                    );
                }
                obj["counterexample"] = serde_json::Value::Object(pairs);
                if let Some(ce) = result.counterexample {
                    obj["counterexample_index"] = serde_json::json!(ce);
                }
            }
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
        _ => {
            if let Some(size) = &stats_value {
                eprintln!(
                    "编码：variables={} clauses={}",
                    size.variables, size.clauses
                );
            }
            if let (Some(nodes), true) = (bdd_nodes, stats) {
                match bdd_order_used {
                    Some(order) => eprintln!("BDD：nodes={nodes} order={order}"),
                    None => eprintln!("BDD：nodes={nodes}"),
                }
            }
            if result.equivalent {
                println!("等价：{left} ≡ {right}  [{alg}]");
            } else if let Some(asg) = &result.counterexample_assignment {
                let detail = asg
                    .variables()
                    .iter()
                    .map(|name| {
                        format!(
                            "{name}={}",
                            asg.get(name).unwrap_or(Ternary::Unknown).as_char()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("不等价：{left} ≢ {right}  [{alg}]");
                if let Some(index) = result.counterexample {
                    println!("反例：{detail}（赋值编号 {index}）");
                } else {
                    println!("反例：{detail}（变量过多，无编号）");
                }
            }
        }
    }
    if result.equivalent {
        Ok(())
    } else {
        std::process::exit(2)
    }
}

/// 解析逗号分隔的变量名，并校验它们都出现在表达式里。
fn parse_var_list(src: &str, all: &[String], flag: &str) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for name in src.split(',') {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if !all.iter().any(|v| v == name) {
            bail!("{flag} 变量 {name} 不在表达式里");
        }
        if !names.iter().any(|v| v == name) {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

fn assignment_text(asg: &Assignment) -> String {
    if asg.is_empty() {
        return "（空）".to_string();
    }
    asg.variables()
        .iter()
        .map(|name| {
            format!(
                "{name}={}",
                asg.get(name).unwrap_or(Ternary::Unknown).as_char()
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn assignment_json(asg: &Assignment) -> serde_json::Value {
    let mut pairs = serde_json::Map::new();
    for name in asg.variables() {
        pairs.insert(
            name.clone(),
            serde_json::Value::String(
                asg.get(&name)
                    .unwrap_or(Ternary::Unknown)
                    .as_char()
                    .to_string(),
            ),
        );
    }
    serde_json::Value::Object(pairs)
}

/// `quantify` 子命令。
struct QuantifyOptions<'a> {
    vars: Option<String>,
    kind: &'a str,
    binary: Option<String>,
    binary_all: bool,
    bdd_order: String,
    bdd_max_nodes: usize,
    json: bool,
}

fn cmd_quantify(expression: &str, alg_name: &str, opts: QuantifyOptions<'_>) -> Result<()> {
    let QuantifyOptions {
        vars,
        kind,
        binary: binary_list,
        binary_all,
        bdd_order,
        bdd_max_nodes,
        json,
    } = opts;
    let Some(kind) = Quantifier::parse(kind) else {
        bail!("未知量化方向 {kind:?}，可选 exists / forall");
    };
    let alg = resolve_algebra(alg_name)?;
    let expr = parse_expr(expression)?;
    let all = expr.variables();
    let quantified = match &vars {
        Some(list) => {
            let names = parse_var_list(list, &all, "--vars")?;
            if names.is_empty() {
                bail!("--vars 不能为空；省略该参数表示量化全部变量");
            }
            names
        }
        None => all.clone(),
    };
    let mut binary = HashSet::new();
    if binary_all {
        binary.extend(all.iter().cloned());
    }
    if let Some(list) = binary_list {
        binary.extend(parse_var_list(&list, &all, "--binary")?);
    }
    let limits = if bdd_max_nodes == 0 {
        BddLimits::UNLIMITED
    } else {
        BddLimits {
            max_nodes: bdd_max_nodes,
        }
    };
    let (result, used_order) = match bdd_order.as_str() {
        "best" => {
            let (result, order) =
                quantify_bdd_best_with_limits(&expr, &quantified, kind, alg, &binary, limits)?;
            (result, order.as_str())
        }
        name => {
            let order =
                VarOrder::parse(name).ok_or_else(|| anyhow::anyhow!("未知 BDD 排序：{name}"))?;
            let vars =
                (order != VarOrder::Appearance).then(|| order_variables(&expr, &expr, order));
            let request = BddRequest {
                order: vars.as_deref(),
                limits,
            };
            (
                quantify_bdd_with(&expr, &quantified, kind, alg, &binary, &request)?,
                order.as_str(),
            )
        }
    };
    let holds = match kind {
        Quantifier::Exists => result.is_satisfiable(),
        Quantifier::Forall => result.is_valid(),
    };
    let witness = (kind == Quantifier::Exists && result.is_satisfiable())
        .then(|| result.witness())
        .flatten();
    let counterexample = (kind == Quantifier::Forall && !result.is_valid())
        .then(|| result.counterexample())
        .flatten();
    if json {
        let mut obj = serde_json::json!({
            "algebra": alg.as_str(),
            "method": "bdd-quantify",
            "bdd_order": used_order,
            "quantifier": kind.as_str(),
            "quantified": quantified,
            "free": result.free_variables(),
            "satisfiable": result.is_satisfiable(),
            "valid": result.is_valid(),
            "nodes": result.nodes(),
        });
        if let Some(asg) = &witness {
            obj["witness"] = assignment_json(asg);
        }
        if let Some(asg) = &counterexample {
            obj["counterexample"] = assignment_json(asg);
        }
        println!("{}", serde_json::to_string_pretty(&obj)?);
    } else {
        let symbol = match kind {
            Quantifier::Exists => "∃",
            Quantifier::Forall => "∀",
        };
        let head = format!(
            "{symbol}{{{}}}. {expression}  [{alg} / bdd-quantify]",
            quantified.join(", ")
        );
        match kind {
            Quantifier::Exists => {
                if let Some(asg) = &witness {
                    println!("可满足：{head}");
                    println!("见证：{}", assignment_text(asg));
                } else {
                    println!("不可满足：{head}");
                }
            }
            Quantifier::Forall => {
                if holds {
                    println!("恒真：{head}");
                } else {
                    println!("不恒真：{head}");
                    if let Some(asg) = &counterexample {
                        println!("反例：{}", assignment_text(asg));
                    }
                }
            }
        }
    }
    if holds { Ok(()) } else { std::process::exit(2) }
}

/// `verify` 子命令。
fn cmd_verify(
    path: &std::path::Path,
    engine: &str,
    bdd_order: &str,
    bdd_max_nodes: usize,
    json: bool,
) -> Result<()> {
    let Some(engine) = Engine::parse(engine) else {
        bail!("未知引擎 {engine:?}，可选 sat / bdd");
    };
    let order = match bdd_order {
        "appearance" => None,
        name => {
            Some(VarOrder::parse(name).ok_or_else(|| anyhow::anyhow!("未知 BDD 排序：{name}"))?)
        }
    };
    let limits = if bdd_max_nodes == 0 {
        BddLimits::UNLIMITED
    } else {
        BddLimits {
            max_nodes: bdd_max_nodes,
        }
    };
    let policy = Policy::load(path)?;
    let assumptions = policy.assumptions.len();
    let results = policy.verify_with_bdd_config(engine, order, limits)?;
    let failed = results.iter().filter(|r| !r.holds).count();
    if json {
        let assertions = results
            .iter()
            .map(|r| {
                let mut obj = serde_json::json!({
                    "name": r.name,
                    "holds": r.holds,
                });
                if let Some(nodes) = r.nodes {
                    obj["nodes"] = serde_json::json!(nodes);
                }
                if let Some(asg) = &r.counterexample {
                    let mut pairs = serde_json::Map::new();
                    for name in asg.variables() {
                        pairs.insert(
                            name.clone(),
                            serde_json::Value::String(
                                asg.get(&name)
                                    .unwrap_or(Ternary::Unknown)
                                    .as_char()
                                    .to_string(),
                            ),
                        );
                    }
                    obj["counterexample"] = serde_json::Value::Object(pairs);
                }
                obj
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "engine": engine.as_str(),
                "assumptions": assumptions,
                "assertions": assertions,
                "failed": failed,
            }))?
        );
    } else {
        for result in &results {
            if result.holds {
                println!("OK     {}", result.name);
            } else {
                println!("FAILED {}", result.name);
                if let Some(asg) = &result.counterexample {
                    let detail = asg
                        .variables()
                        .iter()
                        .map(|name| {
                            format!(
                                "{name}={}",
                                asg.get(name).unwrap_or(Ternary::Unknown).as_char()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("       反例：{detail}");
                }
            }
        }
        println!(
            "{} 条断言，{} 失败  [{}]（假设 {} 条）",
            results.len(),
            failed,
            engine.as_str(),
            assumptions
        );
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
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
    if !equivalent_within(&expr, &simplified, alg, Limits::default())
        .map_err(|e| anyhow::anyhow!("化简复核超出预算：{e}"))?
        .equivalent
    {
        bail!(
            "化简结果语义不等价（内部错误）：{} -> {}",
            expr.to_infix(),
            simplified.to_infix()
        );
    }
    match format {
        Format::Json => {
            let obj = serde_json::json!({
                "original": expr.to_infix(),
                "simplified": simplified.to_infix(),
                "algebra": alg.as_str(),
                "equivalent": true,
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
        _ => {
            println!("原式：{}", expr.to_infix());
            println!("化简：{}", simplified.to_infix());
            println!("语义：{alg}");
            println!("语义等价：是");
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
fn cmd_circuit(
    path: &std::path::Path,
    format: Format,
    alg_name: &str,
    assign: Option<&str>,
) -> Result<()> {
    let alg = resolve_algebra(alg_name)?;
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("无法读取网表文件 {}", path.display()))?;
    let circuit =
        parse_netlist(&text).with_context(|| format!("网表解析失败：{}", path.display()))?;

    if let Some(spec) = assign {
        let parsed = parse_assignment(spec)?;
        let mut assignment: BTreeMap<String, Ternary> = BTreeMap::new();
        for name in parsed.variables() {
            if let Some(value) = parsed.get(&name) {
                assignment.insert(name, value);
            }
        }
        if assignment.is_empty() {
            bail!("--assign 不能为空");
        }
        let prop = trit_circuit::circuit::propagate(&circuit, &assignment, alg)
            .with_context(|| format!("网表传播失败：{}", path.display()))?;
        match format {
            Format::Json => {
                let chars = |m: &BTreeMap<String, Ternary>| {
                    m.iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                serde_json::Value::String(v.as_char().to_string()),
                            )
                        })
                        .collect::<serde_json::Map<String, serde_json::Value>>()
                };
                let obj = serde_json::json!({
                    "circuit": circuit.name,
                    "algebra": alg.as_str(),
                    "assign": chars(&prop.inputs),
                    "signals": chars(&prop.signals),
                    "outputs": chars(&prop.outputs),
                    "origins": prop.origins
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::Array(
                            v.iter().map(|s| serde_json::Value::String(s.clone())).collect())))
                        .collect::<serde_json::Map<String, serde_json::Value>>(),
                    "masked": prop.masked,
                });
                println!("{}", serde_json::to_string_pretty(&obj)?);
            }
            _ => {
                println!("传播：{}  [{}]", path.display(), alg.as_str());
                println!("\n信号值：");
                for (name, value) in &prop.signals {
                    println!("  {name} = {}", value.as_char());
                }
                println!("\n输出：");
                for (name, value) in &prop.outputs {
                    if value.is_unknown() {
                        let origins = prop.origins.get(name).cloned().unwrap_or_default();
                        let list: Vec<&str> = origins.iter().map(|s| s.as_str()).collect();
                        if list.is_empty() {
                            println!("  {name} = X");
                        } else {
                            println!("  {name} = X（X 来源：{}）", list.join(", "));
                        }
                    } else {
                        println!("  {name} = {}", value.as_char());
                    }
                }
                if !prop.masked.is_empty() {
                    println!("\n被遮蔽（输入含 X、输出确定）：{}", prop.masked.join(", "));
                }
            }
        }
        return Ok(());
    }

    if circuit.input_count() > 10 {
        bail!(
            "网表分析最多 10 个主输入（3^11 = 177147 组 × 故障注入），当前 {}",
            circuit.input_count()
        );
    }

    let report = trit_circuit::circuit::analyze(&circuit, alg)
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
                "faults": report.faults
                    .iter()
                    .map(|f| serde_json::json!({
                        "gate": f.gate,
                        "input": f.input,
                        "input_index": f.input_index,
                        "stuck_at": f.stuck_at.as_char().to_string(),
                        "observable": f.observable,
                        "witness": f.witness,
                        "reason": f.reason.map(|r| r.as_str()),
                    }))
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
            let untestable: Vec<&trit_circuit::circuit::Fault> =
                report.faults.iter().filter(|f| !f.observable).collect();
            if !untestable.is_empty() {
                println!("\n不可测故障（stuck-at 无法观测）：");
                for fault in untestable {
                    println!(
                        "  门 {} 的输入 {}[{}] stuck-at {}（{}）",
                        fault.gate,
                        fault.input,
                        fault.input_index,
                        fault.stuck_at.as_char(),
                        fault.reason.map(|r| r.as_str()).unwrap_or("unknown")
                    );
                }
            }
        }
    }
    Ok(())
}

/// `netlist` 子命令：网表文本与 JSON 互转。
fn cmd_netlist(path: &std::path::Path, from: &str, to: &str) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("无法读取网表文件 {}", path.display()))?;
    let circuit = match from {
        "text" => {
            parse_netlist(&text).with_context(|| format!("网表解析失败：{}", path.display()))?
        }
        "json" => trit_circuit::netlist::from_json(&text)
            .with_context(|| format!("JSON 解析失败：{}", path.display()))?,
        other => bail!("未知输入格式 {other:?}，可选 text / json"),
    };
    match to {
        "text" => print!("{}", trit_circuit::netlist::to_netlist(&circuit)),
        "json" => println!("{}", trit_circuit::netlist::to_json(&circuit)?),
        other => bail!("未知输出格式 {other:?}，可选 text / json"),
    }
    Ok(())
}

/// `atpg` 子命令：生成 stuck-at 测试向量与覆盖率。
fn cmd_atpg(path: &std::path::Path, format: Format, alg_name: &str) -> Result<()> {
    let alg = resolve_algebra(alg_name)?;
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("无法读取网表文件 {}", path.display()))?;
    let circuit =
        parse_netlist(&text).with_context(|| format!("网表解析失败：{}", path.display()))?;
    if circuit.input_count() > 10 {
        bail!(
            "ATPG 最多 10 个主输入（2^11 = 2048 个向量 × 故障数），当前 {}",
            circuit.input_count()
        );
    }
    let report = trit_circuit::generate_tests(&circuit, alg)
        .with_context(|| format!("ATPG 失败：{}", path.display()))?;

    match format {
        Format::Json => {
            let obj = serde_json::json!({
                "circuit": circuit.name,
                "algebra": alg.as_str(),
                "total_faults": report.faults.len(),
                "testable": report.testable,
                "coverage": report.coverage,
                "exhaustive": report.exhaustive,
                "optimal": report.optimal,
                "by_gate_kind": report
                    .by_gate_kind
                    .iter()
                    .map(|(kind, stat)| (
                        kind.to_string(),
                        serde_json::json!({
                            "total": stat.total,
                            "testable": stat.testable,
                            "coverage": stat.coverage(),
                        }),
                    ))
                    .collect::<BTreeMap<_, _>>(),
                "patterns": report
                    .patterns
                    .iter()
                    .map(|p| serde_json::json!({
                        "inputs": p
                            .inputs
                            .iter()
                            .map(|(k, v)| (k.clone(), v.as_char().to_string()))
                            .collect::<BTreeMap<_, _>>(),
                        "covers": p.covers,
                    }))
                    .collect::<Vec<_>>(),
                "untestable": report
                    .untestable
                    .iter()
                    .map(|f| serde_json::json!({
                        "gate": f.gate,
                        "input": f.input,
                        "input_index": f.input_index,
                        "stuck_at": f.stuck_at.as_char().to_string(),
                        "reason": f.reason.map(|r| r.as_str()),
                    }))
                    .collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
        Format::Csv => {
            let mut out = String::from("pattern");
            for input in &circuit.inputs {
                out.push(',');
                out.push_str(input.name());
            }
            out.push_str(",covered\n");
            for (index, pattern) in report.patterns.iter().enumerate() {
                out.push_str(&(index + 1).to_string());
                for input in &circuit.inputs {
                    let value = pattern
                        .inputs
                        .get(input.name())
                        .copied()
                        .unwrap_or(Ternary::Unknown);
                    out.push(',');
                    out.push(value.as_char());
                }
                out.push(',');
                out.push_str(&pattern.covers.len().to_string());
                out.push('\n');
            }
            print!("{out}");
        }
        _ => {
            println!(
                "覆盖率 {:.1}%（{}/{}），测试向量 {} 个（{}）",
                report.coverage * 100.0,
                report.testable,
                report.faults.len(),
                report.patterns.len(),
                if report.optimal {
                    "精确最小"
                } else if report.exhaustive {
                    "全向量搜索"
                } else {
                    "见证贪心"
                }
            );
            let kinds = report
                .by_gate_kind
                .iter()
                .map(|(kind, stat)| {
                    format!(
                        "{kind} {}/{}（{:.1}%）",
                        stat.testable,
                        stat.total,
                        stat.coverage() * 100.0
                    )
                })
                .collect::<Vec<_>>()
                .join("，");
            if !kinds.is_empty() {
                println!("按门类型：{kinds}");
            }
            for (i, pattern) in report.patterns.iter().enumerate() {
                let inputs = pattern
                    .inputs
                    .iter()
                    .map(|(name, value)| format!("{name}={}", value.as_char()))
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("#{} {inputs}  → {} 个故障", i + 1, pattern.covers.len());
            }
            if !report.untestable.is_empty() {
                println!("\n不可测故障（{} 个）：", report.untestable.len());
                for fault in &report.untestable {
                    println!(
                        "  门 {} 的输入 {}[{}] stuck-at {}（{}）",
                        fault.gate,
                        fault.input,
                        fault.input_index,
                        fault.stuck_at.as_char(),
                        fault.reason.map(|r| r.as_str()).unwrap_or("unknown")
                    );
                }
            }
        }
    }
    Ok(())
}

/// `belnap` 子命令：Belnap 四值（opt-in）。
fn cmd_belnap(cmd: BelnapCmd) -> Result<()> {
    match cmd {
        BelnapCmd::Eval { expression, assign } => {
            let expr = parse_expr(&expression)?;
            let assignment = parse_b4_assignment(&assign)?;
            let value = trit_belnap::eval(&expr, &assignment);
            let shown: Vec<String> = expr
                .variables()
                .into_iter()
                .map(|var| {
                    let v = assignment.get(&var).copied().unwrap_or(trit_belnap::B4::N);
                    format!("{var}={v}")
                })
                .collect();
            if shown.is_empty() {
                println!("=> {value}");
            } else {
                println!("{}  =>  {value}", shown.join(" "));
            }
        }
        BelnapCmd::Table { expression } => {
            let expr = parse_expr(&expression)?;
            let vars = expr.variables();
            if vars.len() > 6 {
                bail!(
                    "Belnap 真值表最多 6 个变量（4^7 = 16384 行），当前 {}",
                    vars.len()
                );
            }
            let table = trit_belnap::table(&expr);
            println!("{} | 值", table.vars.join(" "));
            for row in &table.rows {
                let cells: Vec<String> = row.values.iter().map(|v| v.to_string()).collect();
                println!("{} | {}", cells.join(" "), row.value);
            }
        }
        BelnapCmd::Equiv { left, right } => {
            let l = parse_expr(&left)?;
            let r = parse_expr(&right)?;
            let mut vars = l.variables();
            for var in r.variables() {
                if !vars.contains(&var) {
                    vars.push(var);
                }
            }
            if vars.len() > 8 {
                bail!(
                    "Belnap 等价最多 8 个变量（4^9 = 262144 组），当前 {}",
                    vars.len()
                );
            }
            match trit_belnap::equivalent(&l, &r) {
                None => println!(
                    "等价（4^{} = {} 组全等）",
                    vars.len(),
                    4usize.pow(vars.len() as u32)
                ),
                Some(counterexample) => {
                    let shown: Vec<String> = vars
                        .iter()
                        .map(|var| {
                            let v = counterexample
                                .get(var)
                                .copied()
                                .unwrap_or(trit_belnap::B4::N);
                            format!("{var}={v}")
                        })
                        .collect();
                    println!("不等价：反例 {}", shown.join(" "));
                    std::process::exit(2);
                }
            }
        }
    }
    Ok(())
}

/// 解析 Belnap 赋值串 `a=T,b=B`（也接受 0/1/? 与 *）。
fn parse_b4_assignment(src: &str) -> Result<BTreeMap<String, trit_belnap::B4>> {
    let mut assignment = BTreeMap::new();
    for part in src.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((name, value)) = part.split_once('=') else {
            bail!("Belnap 赋值格式应为 name=value，收到 {part:?}");
        };
        let name = name.trim();
        if name.is_empty() {
            bail!("Belnap 赋值 {part:?} 缺少变量名");
        }
        let value = value.trim();
        let mut chars = value.chars();
        let ch = chars
            .next()
            .with_context(|| format!("Belnap 赋值 {part:?} 缺少取值"))?;
        if chars.next().is_some() {
            bail!("Belnap 赋值 {part:?} 的取值只能是单字符 T/F/B/N");
        }
        let v = trit_belnap::B4::parse_char(ch)
            .with_context(|| format!("未知四值 {ch:?}，只能是 T/F/B/N（也接受 0/1/?/*）"))?;
        assignment.insert(name.to_string(), v);
    }
    Ok(assignment)
}

/// `repl` 子命令：交互式会话。
fn cmd_repl(alg_name: &str) -> Result<()> {
    let mut alg = resolve_algebra(alg_name)?;
    println!("三值逻辑分析器 REPL  [{alg}]");
    println!("输入表达式查看真值表；`:help` 看命令，`:quit` 退出。");

    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        print!("trit> ");
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
                            alg = a;
                            println!("已切换到 {a}");
                        }
                        Err(e) => println!("{e:#}"),
                    }
                    continue;
                }
                match ExprParser::parse(input) {
                    Ok(expr) => {
                        match truth_table_within(
                            &expr,
                            alg,
                            Limits {
                                max_vars: 10,
                                ..Limits::default()
                            },
                        ) {
                            Ok(tt) => {
                                print!("{}", Format::Table.render_truth_table(&tt, input))
                            }
                            Err(e) => println!("{e}"),
                        }
                    }
                    Err(e) => println!("解析错误：{e}"),
                }
            }
        }
    }
    Ok(())
}
