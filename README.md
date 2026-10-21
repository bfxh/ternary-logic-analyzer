# trit

[![CI](https://github.com/bfxh/trit/actions/workflows/ci.yml/badge.svg)](https://github.com/bfxh/trit/actions/workflows/ci.yml)

Rust 原生的三值推理工具箱：Kleene K3 / Łukasiewicz Ł3 / Bochvar B3 语义、真值表、穷举参考、
dual-rail + SAT 等价判定、组合网表 X 传播；二进制与三值的边界是**零拷贝视图**。

定位与边界见 [docs/DESIGN.md](docs/DESIGN.md)。

## 工作区

| crate | 内容 |
|---|---|
| `trit-core` | `Ternary`、K3/Ł3/B3、AST、解析器、位并行 `TriBit`、位切片 `TriVec` |
| `trit-eval` | 求值、真值表、穷举等价、化简 |
| `trit-sat` | dual-rail + SAT 等价判定与反例 |
| `trit-circuit` | 组合网表 X 传播、stuck-at 可测性 |
| `trit-cli` | `trit` 命令行 |

## 快速开始

```bash
cargo run --release -p trit-cli -- eval "a & b" -s "a=1,b=X"
cargo run --release -p trit-cli -- equiv "a & (b | c)" "(a & b) | (a & c)"
cargo run --release -p trit-cli -- -a lukasiewicz equiv "X => X" "1"
cargo run --release -p trit-cli -- equiv --symbolic "..." "..."   # 大变量数走 SAT
```

`--algebra` / `-a` 选 `kleene`（默认）、`lukasiewicz`、`bochvar`；不等价退出码为 `2`。
穷举默认在 >18 变量时报错并提示 `--symbolic`（`--max-vars N` 可调）。

## 二进制 ↔ 三值：零开销

双导轨 `(ones, unknowns)` 是唯一交换格式：`(0,0)=False`、`(1,0)=True`、`(0,1)=Unknown`。
二进制数据就是 `unknowns` 缺失：

- `Ternary::from_bool` / `to_bool`：标量边界；
- `TriBit::from_binary(u64)` / `as_binary()`：64 路位并行的零开销桥；
- `TriVec::from_binary(Vec<u64>, len)`：**接管**位平面，不复制、不分配 unknown 平面；
  只有写入 `Unknown` 时才惰性分配第二平面。

实测（1,048,576 元素，release，同机单次；`cargo run --release -p trit-core --example bulk_bench`）：

| 操作 | 耗时 | 吞吐 |
|---|---|---|
| `bool[] -> TriVec` 打包 | 0.65 ms | 1.62 G elem/s |
| K3 `AND`（位并行） | 0.033 ms | 31.9 G elem/s |
| `TriVec -> bool[]`（纯二值） | 0.97 ms | 1.08 G elem/s |
| `from_binary` 二进制视图 | 0.002 ms | 仅 move，无复制 |

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

反例编号与 `decode_assignment` 的约定一致。

## 网表

`trit circuit examples/and_gate.net`：输出含 `X` 的输入组合数、各输出值域、冗余输入与 stuck-at 不可测故障。
只支持组合逻辑，不做时序/时钟。

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

`main` 受保护：只接受 PR + 4 个必需 check（三平台 + MSRV 1.88），线性历史。

## 许可

双许可：[MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE)。
