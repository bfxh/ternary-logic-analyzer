# trit 的设计与定位

## 一句话

**可嵌入、可判红的三值推理工具箱**：让 `Unknown` 成为一等公民，并且能在 CI 里对策略/网表/公式给出
确定结论或反例。不做 EDA 全流程，也不做通用模型检验器。

## 系统分层

| 层 | 内容 | 本仓落位 |
|---|---|---|
| L0 语义 | K3 / Ł3 / B3 | `trit-core::algebra` |
| L1 表示 | AST/解析器、真值表、双导轨、位切片 | `trit-core` |
| L2 引擎 | 穷举参考、SAT 等价、化简、网表分析 | `trit-eval` / `trit-sat` / `trit-circuit` |
| L3 应用 | 决策表门禁、硬件 X 检查、教学 | `trit-cli` + 外部 CI |
| L4 工程 | 交叉验证、变异、基准、文档 | 各 crate 的 tests / examples |

## 表示不变量

- 三值量 `(ones, unknowns)`：`(0,0)=False`、`(1,0)=True`、`(0,1)=Unknown`，不允许 `(1,1)`。
- 二进制是 `unknowns = None`（或全零）的特例。`TriVec::from_binary` 接管 `Vec<u64>`，零拷贝。
- `Unknown` 平面惰性分配：纯二值路径不付第二平面的内存/带宽。
- 位级 `TriBit` 与标量 `Ternary` 是同一语义的两种粒度；转换不改变表示。

## 引擎与正确性

- **穷举是 oracle**：`trit-eval::equivalent` 用位并行真值表；`trit-sat` 必须与它逐条对拍。
- **SAT 是规模路径**：按代数真值表逐门加子句，K3/Ł3/B3 共用；反例必须能被 `eval` 复核。
- **网表是独立语义**：组合逻辑、穷举输入；不做时序。
- 回归纪律：任何新语义先加穷举↔SAT 对拍，再加变异（改一个真值表项必须判红）。

## 性能边界

| 路径 | 规模 | 手段 |
|---|---|---|
| 真值表 / 穷举 | ≤18 变量 | 位并行 64 路 |
| SAT 等价 | 数百变量（结构化公式） | dual-rail + CDCL |
| 位切片批量 | 任意长度 | 2 bit/elem、按字运算 |
| 二进制互操作 | 任意长度 | `Option<unknowns>` + move-only 视图 |

## 非目标

- 时序模型检验、CTL/LTL、不动点（TVLA 那条线是另一个产品）。
- 硬件四态 `0/1/X/Z` 与 X-optimism（真值函数三值不够）。
- 通用 SMT / 完整 EDA 仿真。

## 路线

- **v0.2（本版）**：五 crate workspace、二进制零开销桥、SAT 与网表并入。
- **v0.3**：反例最小化或 BDD 后端（二选一，按基准）；`trit verify` 决策表 + GitHub Action 模板。
- **v0.4**：WASM Playground；四值（Belnap）作为 opt-in 语义。

## 参考

- Kleene 1952（K3）；Łukasiewicz；Bochvar 1938（B3）。
- Belnap 1977, *A Useful Four-Valued Logic*。
- Sagiv, Reps, Wilhelm 1999/2002, *Parametric Shape Analysis via 3-Valued Logic*（TVLA）。
- Bruns & Godefroid 1999, *Model Checking Partial State Spaces with 3-Valued Temporal Logics*。
- Chechik et al., *Multi-Valued Symbolic Model-Checking*。
- IEEE 1364 四态与 X-propagation；Bryant 1986（BDD）；Tseitin 1968（CNF）。
