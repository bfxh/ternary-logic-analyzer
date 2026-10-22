//! 三值求值、真值表与化简。

pub mod eval;
pub mod simplify;

pub use eval::{
    Assignment, EquivalenceResult, VarMap, assignment_index, decode_assignment, equivalent,
    equivalent_within, eval, eval_bitwise, eval_within, truth_table, truth_table_within,
};
pub use simplify::{SimplifyOptions, simplify, simplify_with};
