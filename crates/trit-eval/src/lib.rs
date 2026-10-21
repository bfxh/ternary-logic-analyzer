//! 三值求值、真值表与化简。

pub mod eval;
pub mod simplify;

pub use eval::{
    Assignment, EquivalenceResult, VarMap, decode_assignment, equivalent, eval, eval_bitwise,
    truth_table,
};
pub use simplify::{SimplifyOptions, simplify, simplify_with};
