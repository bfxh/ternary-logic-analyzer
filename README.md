# 三值逻辑分析器 (Ternary Logic Analyzer)

[![CI](https://github.com/bfxh/ternary-logic-analyzer/actions/workflows/ci.yml/badge.svg)](https://github.com/bfxh/ternary-logic-analyzer/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE-MIT)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

用 Rust 写的三值逻辑（ternary logic）分析器：求值、完整真值表、逻辑等价判定、代数化简，以及三值门级网表的 `X` 传播与故障可测性分析。

支持三套三值语义：

| 语义 | `X` 的含义 | `X ∧ 0` | `X ∧ 1` | `X ∨ 0` | `X ∨ 1` | `X ⊕ X` | `X ⊗ X` | `X ⇒ X` |
|---|---|---|---|---|---|---|---|---|
| **Kleene** (K3) | 不知道（格序中点） | `0` | `X` | `X` | `1` | `X` | `X` | `X` |
| **Łukasiewicz** (Ł3) | 0 与 1 之间的模糊值 | `0` | `X` | `X` | `1` | `1` | `0` | `1` |
| **Bochvar** (B3) | 毒值（吸收元） | `X` | `X` | `X` | `X` | `X` | `X` | `X` |

> `⊕` = 强加法，Ł3 下为 `min(1, a+b)`；`⊗` = 强合取，Ł3 下为 `max(0, a+b−1)`；
> `⇒` 在 K3 下是材料蕴含 `¬a ∨ b`，在 Ł3 下是 MV 蕴含 `min(1, 1−a+b)`。

## 快速开始

```bash
git clone https://github.com/bfxh/ternary-logic-analyzer.git
cd ternary-logic-analyzer
cargo build --release
```

二进制名为 `tla`：

```bash
# 求值
tla eval "a & b" -s "a=1,b=X"          # => X（Kleene：1 ∧ X = X）

# 真值表
tla table "a & b"
tla table "a & b" -f markdown
tla table "a & b" -f json
tla table "a & b" -f csv

# 等价判定（不等价时退出码为 2，便于脚本判断）
tla equiv "a & b" "b & a"                # => 等价
tla equiv "a & b" "a | b"                # => 不等价 + 反例

# 化简
tla simplify "a & 1 | b & 0"            # => 化简：a；语义等价：是

# 切换语义
tla -a lukasiewicz eval "a ^ b" -s "a=X,b=X"   # => 1（Ł3：X ⊕ X = 1）
tla -a bochvar    eval "a & b" -s "a=0,b=X"    # => X（B3：0 ∧ X = X）

# 网表分析
tla circuit examples/and_gate.net

# 交互式会话
tla repl
```

## 表达式语法

优先级从低到高：

```text
equiv    := implies ( ("≡" | "==") implies )*
implies  := or ( ("=>" | "→") or )*          -- 右结合
or       := xor ( ("∨" | "|" | "||") xor )*
xor      := and ( ("⊕" | "^") and )*
and      := unary ( ("∧" | "&" | "&&") unary )*
unary    := ("!" | "~" | "¬") unary | primary
primary  := "(" expr ")" | 标识符 | 常量
常量     := 0 | 1 | X | x | ? | True | False
```

ASCII 与全角运算符等价，标识符为字母/数字/下划线。

## 作为库使用

```rust
use tla_core::algebra::Algebra;
use tla_core::eval::{equivalent, truth_table};
use tla_core::parser::Parser;

let expr = Parser::parse("a & b | !a").unwrap();

// 完整真值表（3^n 行）
let tt = truth_table(&expr, Algebra::Kleene);
assert_eq!(tt.values.len(), 9);

// 等价判定
let other = Parser::parse("!a | (a & b)").unwrap();
assert!(equivalent(&expr, &other, Algebra::Kleene).equivalent);
```

### 位并行内核

真值表引擎用两个 `u64` 位平面表示三值（`ones` / `unknowns`），
一个机器字同时处理 64 组赋值。表达式编译成位平面上的运算，
因此 `3^n` 行的真值表只需 `⌈3^n / 64⌉` 次位运算即可得到。

`crates/tla-core/src/bits.rs` 里的 `TriBit` 实现了全部算子的位并行版本，
并有属性测试保证它们与标量语义**逐位一致**。

## 网表格式

```text
# 注释以 # 开头
input  a
input  b
output y
output yn

gate AND a b y
not   y yn
```

支持的指令：

| 指令 | 形式 | 说明 |
|---|---|---|
| `input` | `input <信号>` | 声明主输入 |
| `output` | `output <信号>` | 声明主输出 |
| `gate` | `gate <类型> <输入1> <输入2> <输出>` | 类型：`AND` `OR` `XOR` `NAND` `NOR` |
| `not` | `not <输入> <输出>` | 非门 |

分析器报告：

- **输出含未知（X）的组合数** —— 哪些输入组合会让 `X` 传到输出；
- **输出值域** —— 每个主输出可能取到的值；
- **冗余输入** —— 该输入的 stuck-at 故障对本门输出无影响，可从电路中删除；
- **不可测故障** —— 该故障在所有输入组合下都无法被主输出观测到。

## 符号化等价（SAT）

`equiv` 默认是 3^n 穷举，变量一多就慢；`--symbolic` 换成 dual-rail + SAT（纯 Rust 的 `varisat`）：

```bash
tla equiv --symbolic "a & (b | c)" "(a & b) | (a & c)"
```

每个三值量用两根布尔导轨 `(t, x)` 编码（`False=(0,0)`、`True=(1,0)`、`Unknown=(0,1)`），
每个运算符按所选代数的真值表逐门加子句，K3/Ł3/B3 共用同一套编码。
等价判定 = 断言两个根“不相等”，UNSAT 即等价；SAT 时从模型取反例。

实测（同一台机器，单次，n 个变量的 AND 链与自身比较）：

| 变量数 | 穷举 `equiv` | `--symbolic` |
|---|---|---|
| 12 | 0.05 s | 0.02 s |
| 16 | 1.27 s | 0.02 s |
| 18 | 12.6 s | 0.03 s |
| 20 | 默认报错（护栏） | 0.02–0.37 s |
| 50 | 不可行 | 0.03 s |

`equiv` 默认在超过 18 个变量时**报错并提示 `--symbolic`**（避免 3^n 卡死）；`--max-vars N` 可调穷举上限。
反例编号与 `decode_assignment` 的约定一致。

## 项目结构

```text
crates/
  tla-core/          核心库
    src/value.rs      Ternary 三值类型
    src/algebra.rs    Kleene / Łukasiewicz / Bochvar 语义
    src/bits.rs       位并行真值表引擎
    src/ast.rs        表达式 AST
    src/parser.rs     递归下降解析器
    src/eval.rs       求值、真值表、等价判定
    src/simplify.rs   代数化简与语义常量折叠
    src/circuit.rs    网表分析与故障检测
    src/symbolic.rs   dual-rail + SAT 等价判定
    tests/            属性测试与符号化对拍
  tla-cli/           命令行界面
    src/main.rs       六个子命令
    src/output.rs     table / json / csv / markdown 输出
    tests/            端到端集成测试
```

## 开发

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 许可

双许可：[MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE)，由你选择。
