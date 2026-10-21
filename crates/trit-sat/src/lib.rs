//! 三值等价的 dual-rail + SAT 符号化判定。

pub mod symbolic;

pub use symbolic::{SymbolicError, equivalent_symbolic};
