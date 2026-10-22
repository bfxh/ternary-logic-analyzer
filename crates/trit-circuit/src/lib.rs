//! 三值组合网表的 X 传播与 stuck-at 可测性分析。

pub mod atpg;
pub mod circuit;
pub mod netlist;

pub use atpg::{AtpgReport, CoverageStat, TestPattern, generate_tests};
pub use circuit::{
    Circuit, CircuitError, CircuitReport, Fault, Gate, GateKind, Propagation, Signal,
    SimulationRow, UntestableReason, analyze, propagate,
};
pub use netlist::{NetlistError, from_json, parse_netlist, to_json, to_netlist};
