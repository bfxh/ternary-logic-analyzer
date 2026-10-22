# Changelog

## [未发布]

### Added

- `trit atpg <netlist>` 与 `trit_circuit::generate_tests`：门引脚 stuck-at 的测试向量生成
  （贪心集合覆盖）+ 覆盖率报告。
- `Fault::witness`（首个二进制区分向量）、`Fault::reason` 与 `Circuit::decode_binary`。
- `AtpgReport::by_gate_kind`：覆盖率按门类型拆分（table 与 JSON 都输出）。
- BDD 节点预算：`BddLimits`（默认 100 万节点/单个 BDD）与 `equivalent_bdd_with_limits`；超限返回
  `BddError::BudgetExceeded`，构造过程逐步检查。
- BDD 变量排序：`VarOrder`（appearance / reverse / frequency / frequency-asc）、`order_variables`、
  `equivalent_bdd_with_order`、`equivalent_bdd_best`；CLI `equiv --bdd-order`。
- ATPG 精确最小覆盖：故障 ≤ 64、向量 ≤ 256 时用 DFS + 下界剪枝求最小测试集
  （`AtpgReport::optimal`），超预算退回贪心。
- ATPG 覆盖搜索：小规模电路枚举全部 `2^n` 向量做贪心覆盖（`AtpgReport::exhaustive`），
  可用一个向量覆盖多个故障（三个独立 BUF 从 4 个向量降到 2 个）。
- 不可测原因分类：`redundant` / `unobserved` / `masked`，`trit circuit` 与 `trit atpg` 的
  table/JSON 都会输出。

### Changed

- stuck-at 可测性判定改用 `2^n` 二进制测试向量；`X` 输入不是物理测试向量，`analyze` 的
  `observable` 与 ATPG 口径一致。
- 主输入超过 16 时故障分析返回 `CircuitError::TooManyInputs`。

## [0.6.0] - 2026-10-22

### Added

- `trit-core::Limits` + `check_expr` / `check_vars`：库级统一预算（深度 / 节点 / 枚举变量数）。
- `trit-eval`：`eval_within` / `truth_table_within` / `equivalent_within`。
- SAT、BDD、策略入口先做深度/节点检查，超限返回 `Limit` 错误；CLI 与 WASM 共用同一套预算。
- `scripts/mutation_check.py`：12 条关键规则的变异门（12/12 被抓）；新增每周 CI。

### Fixed

- 变异门发现 `strong_and` / `strong_add` 的 K3 断言过弱（只测了 `X⊗X` 与 `1⊗1`），
  补上 `1⊗0 = 0`、`0⊗1 = 0` 的固定用例。

## [0.5.0] - 2026-10-22

### Changed（破坏性）

- `CircuitReport::untestable_faults` 替换为 `faults: Vec<Fault>`：逐**门引脚**、逐极性（SA-0/SA-1）。
  `trit circuit` 的 JSON 字段同步为 `faults`（含 `input_index` / `stuck_at` / `observable`）。

### Added

- `trit-core`：解析嵌套上限 256（`ParseError::TooDeep`）。
- CLI 入口规模护栏：`table` / REPL 最多 10 个变量，网表分析最多 10 个主输入。

### Fixed

- 故障注入从整条 net 改为 gate-pin：多扇出不再串扰，SA-0 / SA-1 分开报告。
- `TriVec::from_ternary` / `and_assign` / `or_assign` 的纯二值路径不再分配 Unknown 临时平面。
- GitHub Action 全部固定到 commit SHA（checkout / rust-toolchain / rust-cache / artifact / gh-release），
  MSRV 1.88 同样固定。
- 文档：DESIGN 路线补 v0.5；CONTRIBUTING 的 `tla-core` 路径改为 `trit-core` / `trit-eval`；
  `bulk_bench` 把缓冲区分配移出 move-only 计时；README 明确双导轨只是进程内表示，不是字节协议。

## [0.4.2] - 2026-10-22

### Fixed

- `trit-circuit`：`analyze` / `propagate` / 故障模拟改用真正的拓扑序（此前把全部门输出预置为已解析，
  等于按声明顺序求值）；主输出必须有驱动。
- `trit-eval`：修正三条不保真化简（`a | (a | b)`、K3 `a ⊕ a`、K3/B3 `a ≡ a`）；
  `semantically_equal` 改用真实 `3^n` 等价，不再比较变量顺序敏感的 64 位指纹。
- `trit-bdd`：`is_valid` 相对合法自由变量域判定；`satisfiable_bdd` 与合法域相交，不再把非法导轨 (1,1)
  当成见证；新增 `premise_counterexample_bdd`。
- `trit-policy`：`assumptions` 按“前提确定为 True”过滤（SAT/BDD 新增 `premise_counterexample*`），
  不再用三值 `premise ⇒ assertion` 恒真替代；schema 拒绝未知字段；空断言不再静默通过。
- `trit-core`：`TriVec::from_binary` 规范化多余字与尾部 padding；`TriBit` 的 B3 或不再产生禁止的 `(1,1)`。
- `trit-cli`：CSV/Markdown 每个变量独立成列（此前整行拼成一格、表头拆两行）；REPL `:algebra` 真正切换语义；
  `simplify` 若结果不等价则非零退出，不再 fail-open。
- `action`：`policy` / `engine` 经 env 传入并校验 engine 白名单，不再把输入拼进 shell 脚本。

## [0.4.1] - 2026-10-22

### Fixed

- `trit-sat` / `trit-bdd` 的二值快速路径把 `⊕` 编成经典异或，与解释器不一致：K3/B3 的 `⊕` 在 `0/1` 上是 `∧`，
  Ł3 上是 `∨`。常量（`1 ^ 1`）与 `--binary` 变量都会触发，例如 `trit equiv --symbolic "1 ^ 1" "1"` 误报不等价。
  SAT 新增 `binary_strong_add`，BDD 的二值分支按代数选 `∧`/`∨`，并加回归测试。
- 新增 `trit-sat` 属性测试：随机公式上 SAT 与 `3^n` 穷举逐例一致，且 SAT 给出的反例必须在解释器下区分两边。

## [0.4.0] - 2026-10-22

### Added

- `trit-wasm` + `playground/`：浏览器里跑求值、三值真值表、等价与 Belnap 四值（wasm-bindgen，`--target web`）。
- `trit-belnap`：Belnap 四值 `T/F/B/N` 的 opt-in 语义（双轨 `(t,f)`、`∧/∨/¬/⇒/⇔`、
  4^n 真值表与逐点等价）；`trit belnap eval|table|equiv`；`sansheng` 再导出。
- 网表指令 `const <0|1|X> <out>` 与 `buf <in> <out>`；`examples/x_mask.net` 演示 `X & 0` / `X | 1` 遮蔽。
- 网表校验：同一信号多驱动（含“既是主输入又被门驱动”）与重复主输入分别报
  `MultipleDrivers` / `DuplicateInput`。
- `trit_circuit::propagate` 与 `trit circuit --assign`：给定一条含 `X` 的主输入向量，单遍拓扑传播，
  报告每根信号取值、主输出上的 `X` 来源，以及被遮蔽（输入含 `X`、输出确定）的门；不枚举 `3^n`。
- `trit_bdd::quantify_bdd` 与 `trit quantify`：三值存在/全称量化（`exists` / `forall`），
  量化域是 trit（`{0,1,X}`），可用 `--binary` 收窄到 `{0,1}`；提供自由变量上的见证/反例与 `--json`。
- `sansheng` 再导出 `Quantifier` / `Quantified` / `quantify_bdd`。

### Fixed

- `Gate::constant` 不再把自己的输出当输入，避免 Const 门被判成组合环。

## [0.3.2] - 2026-10-22

### Added

- `trit verify` 策略支持 `assumptions`：先检查前提可满足，再检查 `assumptions ⇒ assertion` 恒真；
  前提不一致返回 `InconsistentAssumptions`。
- `trit_sat::satisfiable` / `trit_bdd::satisfiable_bdd`：返回可满足见证。

## [0.3.1] - 2026-10-22

### Changed

- Release 产物扩到三平台：`trit-linux-x86_64.tar.gz`、`trit-windows-x86_64.zip`、
  `trit-macos-aarch64.tar.gz`；发布改为 build 矩阵 + 单次 publish，避免并发建 release。

## [0.3.0] - 2026-10-22

### Added

- `trit-core`：`TriVec` 位切片三值向量与 `TriBit::from_binary` / `as_binary`；二进制是
  `unknowns` 缺失的零拷贝视图（1M 元素打包 0.65 ms、K3 AND 0.033 ms）。
- `trit-sat` 二值快速路径：`--binary` / `--binary-all`，纯二值子图走经典 CNF
  （100 变量链子句 3866→598，0.028→0.0147 s）；`--stats` 打印编码规模。
- 充分性反例最小化：`minimal_counterexample` / `--minimize`，对偶 `a==b` 检查保证未列出的变量任取仍区分。
- `trit-bdd`：canonical BDD 等价引擎与 CLI `--bdd`；`--stats` 打印差集节点数。
- `trit-policy` + `trit verify`：JSON 决策表、定义展开、全合法赋值断言检查；失败退出码 1 并给最小反例。
- `action/action.yml`：可复用的 `trit verify` GitHub Action。

### Changed

- 仓库由 `ternary-logic-analyzer` 更名 `trit`，拆成 6 个 crate 的 workspace。
- `EquivalenceResult` 新增 `counterexample_assignment`；编号超出 `u64` 时 `counterexample=None`
  （100 变量不再溢出）。

### Fixed

- Release 工作流打包的二进制名由 `tla` 修正为 `trit`。
