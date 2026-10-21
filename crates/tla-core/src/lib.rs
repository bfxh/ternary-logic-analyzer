//! # tla-core
//!
//! 三值逻辑分析器的核心库。
//!
//! 提供：
//!
//! - [`value::Ternary`]：三值取值 `0 / X / 1`；
//! - [`algebra::Algebra`]：Kleene / Łukasiewicz / Bochvar 三套语义；
//! - [`bits`]：位并行真值表引擎（一个 `u64` 字同时算 64 组赋值）；
//! - [`ast::Expr`] / [`parser::Parser`]：表达式语法树与递归下降解析器；
//! - [`eval`]：单点求值、真值表、等价性判定；
//! - [`simplify`]：代数化简与语义常量折叠；
//! - [`circuit`]：三值门级网表分析与故障（`X` 传播）检测；
//! - [`symbolic`]：dual-rail 编码 + SAT 的符号化等价判定。

#![deny(missing_docs)]
#![warn(clippy::all)]

pub mod algebra;
pub mod ast;
pub mod bits;
pub mod circuit;
pub mod eval;
pub mod parser;
pub mod simplify;
pub mod symbolic;
pub mod value;

pub use algebra::Algebra;
pub use ast::Expr;
pub use bits::TruthTable;
pub use parser::Parser;
pub use symbolic::{SymbolicError, equivalent_symbolic};
pub use value::Ternary;
