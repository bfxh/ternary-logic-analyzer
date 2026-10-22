# 贡献指南

感谢参与三值逻辑分析器。

## 开发环境

- Rust 1.88 及以上（stable；MSRV 由 dev-dependency `proptest` 决定）
- `cargo fmt`、`cargo clippy` 组件

```bash
rustup component add rustfmt clippy
```

## 提交前必过门禁

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

三条命令必须全部通过（退出码 0）才允许提交。

## 代码约定

- 公共 API 需要文档注释（`#![deny(missing_docs)]`）。
- 语义相关代码必须注明适用的是哪套语义（Kleene / Łukasiewicz / Bochvar）。
  **三套语义对 `X` 的处理不同，规则不可混用**：
  - K3 / Ł3 的 `∧` `∨` 是格序的 min / max，`X` 是中间元；
  - B3 的 `X` 是毒值，吸收一切，**吸收律与分配律都不成立**；
  - Ł3 的 `X ⊕ X = 1`、`X ⊗ X = 0`、`X ⇒ X = 1`；
  - **矛盾律 `a ∧ ¬a = 0` 在三套语义下都不成立**（`a = X` 时 `X ∧ X = X`）。
- 位并行实现（`crates/trit-core/src/bits.rs`）改动后，必须有属性测试保证
  与标量语义逐位一致（见 `crates/trit-eval/tests/properties.rs`）。
- 化简规则改动后，必须验证**不改变语义**（`simplify_preserves_semantics`）。

## 提交信息

采用 Conventional Commits：

```text
feat(core): 支持强合取 ⊗ 运算
fix(parser): 修正右结合蕴含的优先级
docs(readme): 补充网表格式说明
test(circuit): 增加组合环检测用例
```

## 报告问题

请在 [Issues](https://github.com/bfxh/ternary-logic-analyzer/issues) 中说明：

- 使用的三值语义（Kleene / Łukasiewicz / Bochvar）；
- 最小复现输入（表达式或网表）；
- 实际输出与期望输出。
