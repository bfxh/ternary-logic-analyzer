//! 三值组合网表的 X 传播与 stuck-at 可测性分析。

pub mod circuit;

pub use circuit::{
    Circuit, CircuitError, CircuitReport, Gate, GateKind, Propagation, Signal, SimulationRow,
    analyze, propagate,
};
