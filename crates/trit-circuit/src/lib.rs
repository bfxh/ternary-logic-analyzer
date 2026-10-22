//! 三值组合网表的 X 传播与 stuck-at 可测性分析。

pub mod atpg;
pub mod circuit;

pub use atpg::{AtpgReport, TestPattern, generate_tests};
pub use circuit::{
    Circuit, CircuitError, CircuitReport, Fault, Gate, GateKind, Propagation, Signal,
    SimulationRow, analyze, propagate,
};
