//! 三生（Sansheng）：三值推理工具箱的伞包。
//!
//! 内部实现分布在 `trit-*` crate；这里提供统一入口：
//!
//! ```
//! use sansheng::{equivalent_symbolic, Algebra, Parser};
//! let a = Parser::parse("a & (b | c)").unwrap();
//! let b = Parser::parse("(a & b) | (a & c)").unwrap();
//! assert!(equivalent_symbolic(&a, &b, Algebra::Kleene).unwrap().equivalent);
//! ```
//!
//! 量化按 **trit** 取值（`{0,1,X}`）解释，而不是按两根布尔导轨：
//!
//! ```
//! use std::collections::HashSet;
//! use sansheng::{Algebra, Parser, Quantifier, quantify_bdd};
//! let e = Parser::parse("a | !a").unwrap();
//! let vars = e.variables();
//! let q = quantify_bdd(&e, &vars, Quantifier::Forall, Algebra::Kleene, &HashSet::new()).unwrap();
//! assert!(!q.is_valid(), "三值域下 a=X 时 a|!a 不为真");
//! ```

pub use trit_bdd::{
    BddError, BddVerdict, Quantified, Quantifier, VarOrder, equivalent_bdd, equivalent_bdd_best,
    equivalent_bdd_with_order, order_variables, quantify_bdd, satisfiable_bdd,
};
pub use trit_belnap::{
    B4, Row as BelnapRow, Table as BelnapTable, equivalent as equivalent_belnap,
    eval as eval_belnap, table as table_belnap,
};
pub use trit_circuit::{
    AtpgReport, Circuit, CircuitError, CircuitReport, CoverageStat, Fault, Gate, GateKind,
    Propagation, Signal, SimulationRow, TestPattern, UntestableReason, analyze, generate_tests,
    propagate,
};
pub use trit_core::{Algebra, BitPlane, Expr, Parser, Ternary, TriBit, TriVec, TruthTable};
pub use trit_eval::{
    Assignment, EquivalenceResult, VarMap, assignment_index, decode_assignment, equivalent, eval,
    eval_bitwise, truth_table,
};
pub use trit_policy::{Assertion, AssertionResult, Engine, Policy, PolicyError};
pub use trit_sat::{
    EncodingSize, SymbolicError, encoding_size, equivalent_symbolic, equivalent_symbolic_with,
    minimal_counterexample, satisfiable,
};
