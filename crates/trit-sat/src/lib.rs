//! 三值等价的 dual-rail + SAT 符号化判定。

pub mod symbolic;

pub use symbolic::{
    EncodingSize, SymbolicError, encoding_size, equivalent_symbolic, equivalent_symbolic_with,
    minimal_counterexample, premise_counterexample, satisfiable,
};
