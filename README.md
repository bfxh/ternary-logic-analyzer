# trit

[![CI](https://github.com/bfxh/trit/actions/workflows/ci.yml/badge.svg)](https://github.com/bfxh/trit/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/sansheng.svg)](https://crates.io/crates/sansheng)
[![docs.rs](https://docs.rs/sansheng/badge.svg)](https://docs.rs/sansheng)

Rust 原生的三值推理工具箱：Kleene K3 / Łukasiewicz Ł3 / Bochvar B3 语义、真值表、穷举参考、
dual-rail + SAT 等价判定、组合网表 X 传播；二进制与三值的边界是**零拷贝视图**。

定位与边界见 [docs/DESIGN.md](docs/DESIGN.md)。

## 预编译产物

GitHub Releases（`v0.3.1` 起）提供：

| 平台 | 产物 |
|---|---|
| Linux x86_64 | `trit-linux-x86_64.tar.gz` |
| Windows x86_64 | `trit-windows-x86_64.zip` |
| macOS arm64 | `trit-macos-aarch64.tar.gz` |
| 校验 | `SHA256SUMS` |

## 工作区

| crate | 内容 |
|---|---|
| `trit-core` | `Ternary`、K3/Ł3/B3、AST、解析器、位并行 `TriBit`、位切片 `TriVec` |
| `trit-eval` | 求值、真值表、穷举等价、化简 |
| `trit-sat` | dual-rail + SAT 等价判定与反例 |
| `trit-circuit` | 组合网表 X 传播、stuck-at 可测性 |
| `trit-belnap` | Belnap 四值（T/F/B/N）的 opt-in 语义 |
| `trit-wasm` | WASM 绑定：给 `playground/` 的求值、真值表、等价 |
| `trit-bdd` | BDD（canonical）等价判定、三值量化 |
| `trit-policy` | 三值决策表的构建期验证 |
| `sansheng` | 三生：统一入口伞包（re-export 以上能力） |
| `trit-cli` | `trit` 命令行 |

## 三生（sansheng）

crates.io 包名用拼音 `sansheng`（`三生` 不能直接作 crate 名）。伞包统一暴露核心 API：

```toml
[dependencies]
sansheng = "0.4"
```

```rust
use sansheng::{equivalent_symbolic, Algebra, Parser};
```

CLI 仍是 `trit`（`cargo install trit-cli` 或下载 Release 二进制）。

API 文档：<https://docs.rs/sansheng>。

## 快速开始

```bash
cargo run --release -p trit-cli -- eval "a & b" -s "a=1,b=X"
cargo run --release -p trit-cli -- equiv "a & (b | c)" "(a & b) | (a & c)"
cargo run --release -p trit-cli -- -a lukasiewicz equiv "X => X" "1"
cargo run --release -p trit-cli -- equiv --symbolic "..." "..."   # 大变量数走 SAT
cargo run --release -p trit-cli -- equiv --symbolic --binary-all --stats "..." "..."
cargo run --release -p trit-cli -- equiv --symbolic --minimize "a & (b | !b)" "0"
cargo run --release -p trit-cli -- equiv --bdd "a & (b | c)" "(a & b) | (a & c)"
cargo run --release -p trit-cli -- quantify "a | !a" --vars a --kind forall
cargo run --release -p trit-cli -- verify examples/policy.json
```

`--algebra` / `-a` 选 `kleene`（默认）、`lukasiewicz`、`bochvar`；不等价退出码为 `2`。
穷举默认在 >18 变量时报错并提示 `--symbolic`（`--max-vars N` 可调）。

## 二进制 ↔ 三值：零开销

双导轨 `(ones, unknowns)` 是**进程内表示**：`(0,0)=False`、`(1,0)=True`、`(0,1)=Unknown`，禁止 `(1,1)`。
它**不是**跨语言/持久化的字节协议：没有头部、版本、字节序或 padding 约定；CLI 的 JSON 也没有 `schema_version`。
需要跨进程交换时请显式定义自己的格式，不要把 `Vec<u64>` 的内存布局或 Serde 枚举标签当协议。
二进制数据就是 `unknowns` 缺失：

- `Ternary::from_bool` / `to_bool`：标量边界；
- `TriBit::from_binary(u64)` / `as_binary()`：64 路位并行的零开销桥；
- `TriVec::from_binary(Vec<u64>, len)`：**接管**位平面（多余字截断、尾部掩码），不复制、不分配 unknown 平面；
  只有写入 `Unknown` 时才惰性分配第二平面。

实测（1,048,576 元素，release，同机单次；`cargo run --release -p trit-core --example bulk_bench`）：

| 操作 | 耗时 | 吞吐 |
|---|---|---|
| `bool[] -> TriVec` 打包 | 0.65 ms | 1.62 G elem/s |
| K3 `AND`（位并行） | 0.033 ms | 31.9 G elem/s |
| `TriVec -> bool[]`（纯二值） | 0.97 ms | 1.08 G elem/s |
| `from_binary` 二进制视图 | 0.002 ms | 仅 move，无复制 |

## 资源预算与变异门

所有入口共用 `trit_core::Limits`（默认：深度 256 / 节点 100 000 / 枚举 12 变量）：

- `check_expr` 迭代遍历 AST，`check_vars` 管 `3^n` 枚举；
- `truth_table_within` / `eval_within` / `equivalent_within`：`trit-eval` 的带预算版本；
- SAT / BDD / 策略入口先 `check_expr` 再编码，超限返回专门的 `Limit` 错误；
- CLI 在默认值上收紧：`table` / REPL 与 WASM 只允许 10 个变量，网表分析最多 10 个主输入。

`python scripts/mutation_check.py` 对 12 条关键规则做单点文本变异并跑对应回归，
测试必须变红才算抓住（当前 12/12）；`.github/workflows/mutation.yml` 每周跑一次。

## 符号化等价（SAT）

每个 trit 用两根布尔导轨，按所选代数的真值表逐门加子句，K3/Ł3/B3 共用编码。
等价判定 = 断言两个根不相等，UNSAT 即等价；SAT 时从模型还原反例。

同机单次（n 个变量的 AND 链与自身比较）：

| 变量数 | 穷举 | `--symbolic` |
|---|---|---|
| 12 | 0.05 s | 0.016 s |
| 18 | 12.6 s | 0.016 s |
| 20 | 护栏报错 | 0.017 s |
| 50 | 不可行 | 0.020 s |
| 100 | 不可行 | 0.025 s |

纯二值公式可声明 `--binary-all`（或 `--binary a,b`）：二值子图走经典 CNF，不再付 18 条三值子句。
100 变量 AND 链实测：**3866 → 598 条子句，0.028 s → 0.0147 s**；`--stats` 打印编码规模。

反例编号与 `decode_assignment` 的约定一致。

**反例最小化**（`--minimize`）：从完整模型出发，逐个试删变量；用“断言两边相等”的对偶编码检查，
删掉后若仍可相等就必须保留。得到的是**充分**部分反例——未列出的变量在各自域内任取，两边仍不相等；
不会退化成“只保证存在某个补全”的空反例。

## BDD 引擎（`--bdd`）

第三条等价路径：把双导轨直接建成 BDD，差集 BDD 为 false 即等价。BDD 是 canonical 的，
二值变量只建一根导轨；`--stats` 打印差集节点数。与 SAT 互斥（`--bdd` / `--symbolic` 二选一）。

同机单次（n 个变量的 AND 链与自身比较）：

| 变量数 | SAT | BDD |
|---|---|---|
| 50 | 0.022 s | 0.025 s |
| 100 | 0.028 s | 0.047 s |
| `(a & b) \| (c & d)` | 0.016 s | 0.016 s |

结论：这批结构化小公式上 SAT 略快；BDD 的价值在 canonical 表示与可枚举/可组合的图结构，
不是“无条件更快”。变量顺序、函数结构都会影响 BDD 规模，极端函数会指数爆炸——按基准选引擎。

变量顺序可用 `--bdd-order` 选：`appearance`（默认）、`reverse`、`frequency`、`frequency-asc`、
`best`（试四种取差集 BDD 节点最少者）。`--stats` 会打印所选顺序与节点数。
BDD 节点预算默认 100 万（`BddLimits::default()`），任何一次构造超过上限即返回
`BddError::BudgetExceeded`，不会无限膨胀；库调用可用 `equivalent_bdd_with_limits` 收紧或
`BddLimits::UNLIMITED` 关闭。

## 量化（`trit quantify`）

对三值变量做存在/全称量化。量化的对象是 **trit**（`{0,1,X}`），不是两根布尔导轨：

- `∃x. f`：存在某个合法 trit 取值使 `f` **确定为 True**；
- `∀x. f`：对所有合法 trit 取值 `f` 都确定为 True；
- `--binary x` 把该变量的量化域收窄到 `{0,1}`；自由变量始终按三值域约束。

```bash
$ trit quantify "a & b" --vars a
可满足：∃{a}. a & b  [kleene / bdd-quantify]
见证：b=1

$ trit quantify "a | !a" --vars a --kind forall
不恒真：∀{a}. a | !a  [kleene / bdd-quantify]
反例：（空）

$ trit quantify "a | !a" --vars a --kind forall --binary a
恒真：∀{a}. a | !a  [kleene / bdd-quantify]
```

性质成立退出 `0`，不成立退出 `2`；`--json` 输出 `satisfiable` / `valid` / `free` /
`witness` / `counterexample` / `nodes`。被量化变量已被消去，见证与反例只含自由变量。
省略 `--vars` 即量化全部变量，结果退化为一个布尔判定。

## 四值（Belnap，opt-in）

`trit belnap` 是 K3/Ł3/B3 之外的另一套语义：`T` / `F` / `B`（both）/ `N`（neither）。
它不改变三值的任何行为，只存在于 `trit-belnap`。

```bash
trit belnap eval "a & b" -s "a=N,b=B"     # F：neither 与 both 冲突
trit belnap table "a & b"                  # 16 行
trit belnap equiv "!(a & b)" "!a | !b"     # 等价（4^2 = 16 组全等）
trit belnap equiv "a | !a" "1"             # 不等价，反例 a=N，退出码 2
```

- `∧`：`t = t₁∧t₂`、`f = f₁∨f₂`，所以 `N ∧ B = F`；`∨`：`t = t₁∨t₂`、`f = f₁∧f₂`，所以 `N ∨ B = T`。
- 蕴含取 `¬a ∨ b`；等价取 `(a ⇒ b) ∧ (b ⇒ a)`。
- `⊕` 在 FOUR 下没有公认定义，这里取 `¬(a ⇔ b)`（`B ⊕ B = B`），不要按经典异或理解。
- 表达式常量 `X` 映射为 `N`，`B` 只能通过赋值/真值表枚举出现。三值双导轨禁止 `(1,1)`，四值正是放开它。
- 真值表最多 6 个变量（4^7 = 16384 行），等价判定最多 8 个变量。

## WASM Playground

`playground/` 是浏览器端的同一套 Rust 引擎（`trit-wasm`，wasm-bindgen）。
三值面板支持求值 / 真值表 / 等价，四值面板支持 Belnap 求值与真值表。

```bash
rustup target add wasm32-unknown-unknown
wasm-pack build crates/trit-wasm --target web --out-dir ../../playground/pkg
python -m http.server 8765 --directory playground
# 打开 http://127.0.0.1:8765/
```

`playground/pkg/` 是生成物，不入库；CI 只做 `cargo build -p trit-wasm --target wasm32-unknown-unknown`。

## 网表

`trit circuit examples/and_gate.net`：输出含 `X` 的输入组合数、各输出值域、冗余输入与 stuck-at 不可测故障。
只支持组合逻辑，不做时序/时钟。

`trit atpg examples/and_gate.net`：对每个门引脚的 stuck-at 故障生成**二进制**测试向量（贪心集合覆盖），
输出覆盖率、向量数与不可测故障。可测性判定基于 `2^n` 二进制向量；`X` 输入不是物理测试向量，不计入。
不可测故障会标注原因：`redundant`（固定该引脚不改变门输出）、`unobserved`（到不了主输出）、
`masked`（到得了但被遮蔽）。
覆盖率同时按门类型拆分（`按门类型：AND 4/4（100.0%）…`），JSON 里是 `by_gate_kind`。
故障数 × `2^n` ≤ 50000 时会枚举全部向量再做贪心覆盖（`exhaustive`），否则退回每个故障的首个见证。
故障数 ≤ 64 且向量数 ≤ 256 时再跑精确最小覆盖（`optimal`，DFS + 下界剪枝，10 万节点预算）。

单向量传播（不枚举 3^n）：`trit circuit examples/and_gate.net --assign "a=X,b=0"`。
输出每根信号取值、主输出上 `X` 的来源主输入，以及被遮蔽的门（`X & 0 = 0`、`X | 1 = 1`；
Ł3 的 `X ⊕ X = 1` 也算“确定”）。`--assign` 必须给全所有主输入，缺一个直接报错。

网表指令：`input` / `output` / `gate <AND|OR|XOR|NAND|NOR> <a> <b> <out>` / `not <a> <out>` /
`buf <a> <out>` / `const <0|1|X> <out>`。`examples/x_mask.net` 是 `X & 0`、`X | 1` 遮蔽的最小例子。
同一信号被多个源驱动、或主输入重复声明，都会在分析前报错。

## 构建期验证（`trit verify`）

把决策表写成 JSON，断言必须在**所有合法赋值**上恒真；失败时给出充分最小反例，退出码 1。

```json
{
  "algebra": "kleene",
  "binary": ["infeasible", "contended"],
  "definitions": { "reject": "infeasible | (hopeless & contended)" },
  "assertions": [
    { "name": "硬约束", "expr": "infeasible => reject" },
    { "name": "空槽不拒", "expr": "(!contended & !infeasible) => !reject" }
  ]
}
```

```bash
$ trit verify examples/policy.json
OK     hard constraint: infeasible must reject
OK     free slot must not reject on unknown hopelessness
2 条断言，0 失败  [sat]
```

- `binary` 里的变量只在 0/1 上取值；其余变量按三值（含 X）检查——所以断言比二值布尔更强。
- 可选 `assumptions`：前提必须同时**可满足**，断言只在满足前提的赋值上检查；前提不一致直接判错。
  注意前提不会把 X 变成 0/1，需要二值语义就写进 `binary`。
- `--engine bdd` 切 canonical BDD；`--json` 给 CI 用。
- 可复用 Action（在目标仓里）：

```yaml
- uses: actions/checkout@v4
- uses: bfxh/trit/action@main
  with:
    policy: policy.json
    engine: sat
```

首次运行会从源码构建 `trit`；生产建议 pin 到 tag/SHA。

## 范围

- 只做**真值函数**三值逻辑；硬件四态 `0/1/X/Z` 与 X-optimism 不在范围内。
- 不做时序模型检验、通用 SMT、完整硬件仿真。
- 穷举参考是逐赋值的，18 变量约 12.6 s，仅用于小规模对拍。

## 开发

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI 另有三道门：`docs`（rustdoc `-D warnings`）、`action`（用 `examples/policy.json` 跑复合 Action 自测）、
`MSRV`（1.88，`--locked`）；Release 先过 `guard`：tag 去掉 `v` 后必须等于 `trit-cli` 的版本。

`main` 受保护：只接受 PR + 4 个必需 check（三平台 + MSRV 1.88），线性历史。

## 许可

双许可：[MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE)。
